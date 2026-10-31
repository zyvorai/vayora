import Foundation
import SwiftUI
import VayoraKit

struct Txn: Codable, Identifiable, Equatable {
    enum Status: String, Codable { case success, failed }
    var id = UUID()
    var payee: String
    var name: String
    var amountMinor: Int64
    var note: String
    var status: Status
    var date = Date()
    var money: Money { Money(minor: amountMinor) }
}

/// The payer's local state. The wallet is **test money kept on this phone**: it is not a bank account.
@MainActor
final class Store: ObservableObject {
    @Published var lastError = ""
    @Published var serverURL: String { didSet { UserDefaults.standard.set(serverURL, forKey: "serverURL") } }
    @Published var balanceMinor: Int64 { didSet { UserDefaults.standard.set(balanceMinor, forKey: "balance") } }
    @Published var history: [Txn] { didSet { save() } }
    @Published var testPIN: String { didSet { UserDefaults.standard.set(testPIN, forKey: "testPIN") } }

    static let startingBalance: Int64 = 10_000_00

    init() {
        let d = UserDefaults.standard
        serverURL = d.string(forKey: "serverURL") ?? "http://127.0.0.1:8080"
        balanceMinor = d.object(forKey: "balance") as? Int64 ?? Store.startingBalance
        testPIN = d.string(forKey: "testPIN") ?? "1234"
        history = (d.data(forKey: "history")).flatMap { try? JSONDecoder().decode([Txn].self, from: $0) } ?? []
    }

    private func save() {
        if let d = try? JSONEncoder().encode(history.prefix(200).map { $0 }) { UserDefaults.standard.set(d, forKey: "history") }
    }

    var client: VayoraClient? { URL(string: serverURL).map { VayoraClient(baseURL: $0) } }
    var balance: Money { Money(minor: balanceMinor) }

    var spentThisMonth: Int64 {
        let cal = Calendar.current
        return history.filter { $0.status == .success && cal.isDate($0.date, equalTo: Date(), toGranularity: .month) }
            .reduce(0) { $0 + $1.amountMinor }
    }

    func record(_ t: Txn) {
        history.insert(t, at: 0)
        if t.status == .success { balanceMinor -= t.amountMinor }
    }

    func resetWallet() { balanceMinor = Store.startingBalance; history = [] }

    /// Pay a scanned payload through the simulated payer. Returns the final state; a lost reply is
    /// reported as `.unknown` so the screen says "checking" instead of guessing.
    enum Outcome: Equatable { case paid, declined(String), unknown, notAvailable }

    func pay(_ p: UPIPayload, amount: Money) async -> Outcome {
        guard amount.minor <= balanceMinor else { return .declined("Not enough test balance") }
        guard let c = client else { return .notAvailable }
        do {
            let r = try await c.simPay(upiURI: rebuildURI(p, amount: amount))
            return r.status == "captured" ? .paid : .declined("Payment was not completed")
        } catch let e as APIError where e.status == 404 {
            return .notAvailable
        } catch let e as APIError where e.status == 409 || e.status == 422 {
            return .declined(e.message)
        } catch {
            lastError = "\(error) [url: \(serverURL)]"
            return .unknown
        }
    }

    private func rebuildURI(_ p: UPIPayload, amount: Money) -> String {
        var c = URLComponents(string: "upi://pay")!
        c.queryItems = [.init(name: "pa", value: p.payee), .init(name: "pn", value: p.name),
                        .init(name: "am", value: amount.decimalString), .init(name: "cu", value: "INR"),
                        .init(name: "tn", value: p.note)]
        return c.string ?? ""
    }
}
