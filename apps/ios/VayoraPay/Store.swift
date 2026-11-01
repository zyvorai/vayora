import Foundation
import SwiftUI
import VayoraKit

struct Txn: Codable, Identifiable, Equatable, Hashable {
    enum Status: String, Codable { case success, failed }
    var id = UUID()
    var payee: String
    var name: String
    var amountMinor: Int64
    var note: String
    var status: Status
    var date = Date()
    /// Added later, so older saved history (without these) still loads.
    var kind: String? = nil          // "transfer" for a person-to-person test transfer, nil for a merchant payment
    var category: String? = nil
    var noPIN: Bool? = nil
    var money: Money { Money(minor: amountMinor) }
    var isTransfer: Bool { kind == "transfer" }
    var isAutoPay: Bool { kind == "autopay" }
}

/// An AutoPay this phone approved. The debits are what the (simulated) bank reported, kept so a poll never takes
/// the same one from the wallet twice.
struct ApprovedMandate: Codable, Identifiable, Equatable {
    var id: String
    var merchant: String
    var payee: String
    var capMinor: Int64
    var frequency: String
    var until: String?
    var status: String
    var debits: [PayerDebit] = []
}

/// The payer's local state. The wallet is **test money kept on this phone**: it is not a bank account.
@MainActor
final class Store: ObservableObject {
    @Published var lastError = ""
    /// Kept in the Keychain as well, because iOS keeps Keychain items when an app is deleted and reinstalled.
    @Published var serverURL: String { didSet { UserDefaults.standard.set(serverURL, forKey: "serverURL"); Keychain.set(serverURL, for: "serverURL") } }
    @Published var balanceMinor: Int64 { didSet { UserDefaults.standard.set(balanceMinor, forKey: "balance") } }
    @Published var history: [Txn] { didSet { save() } }
    @Published var testPIN: String { didSet { UserDefaults.standard.set(testPIN, forKey: "testPIN") } }
    @Published var circles: [CircleRecord] { didSet { if let d = try? JSONEncoder().encode(circles) { UserDefaults.standard.set(d, forKey: "circles") } } }
    @Published var autoPays: [ApprovedMandate] { didSet { if let d = try? JSONEncoder().encode(autoPays) { UserDefaults.standard.set(d, forKey: "autoPays") } } }
    @Published var payees: PayeeBook { didSet { if let d = try? JSONEncoder().encode(payees) { UserDefaults.standard.set(d, forKey: "payees") } } }
    @Published var reminders: [BillReminder] { didSet { if let d = try? JSONEncoder().encode(reminders) { UserDefaults.standard.set(d, forKey: "reminders") }; Task { await ReminderScheduler.sync(reminders, scheduled: scheduled) } } }
    @Published var scheduled: [ScheduledPayment] { didSet { if let d = try? JSONEncoder().encode(scheduled) { UserDefaults.standard.set(d, forKey: "scheduled") }; Task { await ReminderScheduler.sync(reminders, scheduled: scheduled) } } }
    @Published var splits: [SplitRecord] { didSet { if let d = try? JSONEncoder().encode(splits.prefix(50).map { $0 }) { UserDefaults.standard.set(d, forKey: "splits") } } }
    @Published var budgets: Budgets { didSet { if let d = try? JSONEncoder().encode(budgets) { UserDefaults.standard.set(d, forKey: "budgets") } } }
    @Published var categoryMemory: CategoryMemory { didSet { if let d = try? JSONEncoder().encode(categoryMemory) { UserDefaults.standard.set(d, forKey: "categoryMemory") } } }
    @Published var vault: Vault { didSet { if let d = try? JSONEncoder().encode(vault) { UserDefaults.standard.set(d, forKey: "vault") } } }
    @Published var jars: JarBook { didSet { if let d = try? JSONEncoder().encode(jars) { UserDefaults.standard.set(d, forKey: "jars") } } }
    @Published var smallPayments: Bool { didSet { UserDefaults.standard.set(smallPayments, forKey: "smallPayments") } }
    @Published var myName: String { didSet { UserDefaults.standard.set(myName, forKey: "myName") } }
    /// Where split requests ask to be paid. Made once per install; a test ID, not a bank account.
    let myVPA: String

    static let startingBalance: Int64 = 10_000_00

    init() {
        let d = UserDefaults.standard
        let saved = Keychain.get("serverURL") ?? d.string(forKey: "serverURL") ?? "http://127.0.0.1:8080"
        serverURL = saved
        // an address saved by an older build lives only in UserDefaults: copy it to the Keychain so a reinstall keeps it
        if URL(string: saved)?.host != "127.0.0.1", Keychain.get("serverURL") == nil { Keychain.set(saved, for: "serverURL") }
        balanceMinor = d.object(forKey: "balance") as? Int64 ?? Store.startingBalance
        testPIN = d.string(forKey: "testPIN") ?? "1234"
        history = (d.data(forKey: "history")).flatMap { try? JSONDecoder().decode([Txn].self, from: $0) } ?? []
        scheduled = (d.data(forKey: "scheduled")).flatMap { try? JSONDecoder().decode([ScheduledPayment].self, from: $0) } ?? []
        splits = (d.data(forKey: "splits")).flatMap { try? JSONDecoder().decode([SplitRecord].self, from: $0) } ?? []
        categoryMemory = (d.data(forKey: "categoryMemory")).flatMap { try? JSONDecoder().decode(CategoryMemory.self, from: $0) } ?? CategoryMemory()
        vault = (d.data(forKey: "vault")).flatMap { try? JSONDecoder().decode(Vault.self, from: $0) } ?? Vault()
        jars = (d.data(forKey: "jars")).flatMap { try? JSONDecoder().decode(JarBook.self, from: $0) } ?? JarBook()
        budgets = (d.data(forKey: "budgets")).flatMap { try? JSONDecoder().decode(Budgets.self, from: $0) } ?? Budgets()
        circles = (d.data(forKey: "circles")).flatMap { try? JSONDecoder().decode([CircleRecord].self, from: $0) } ?? []
        autoPays = (d.data(forKey: "autoPays")).flatMap { try? JSONDecoder().decode([ApprovedMandate].self, from: $0) } ?? []
        payees = (d.data(forKey: "payees")).flatMap { try? JSONDecoder().decode(PayeeBook.self, from: $0) } ?? PayeeBook()
        reminders = (d.data(forKey: "reminders")).flatMap { try? JSONDecoder().decode([BillReminder].self, from: $0) } ?? []
        smallPayments = d.bool(forKey: "smallPayments")
        myName = d.string(forKey: "myName") ?? "Me"
        if let v = d.string(forKey: "myVPA") { myVPA = v } else {
            let v = "pay-" + String(UUID().uuidString.prefix(6)).lowercased() + "@" + PayeeInput.phoneDomain
            d.set(v, forKey: "myVPA"); myVPA = v
        }
    }

    static let smallPolicy = SmallPaymentPolicy()

    /// What went out today without a PIN, so the daily limit holds.
    var noPINSpentToday: Int64 {
        let cal = Calendar.current
        return history.filter { $0.status == .success && $0.noPIN == true && cal.isDateInToday($0.date) }.reduce(0) { $0 + $1.amountMinor }
    }
    func needsPIN(for amount: Money) -> Bool { Store.smallPolicy.needsPIN(amount: amount, noPINSpentTodayMinor: noPINSpentToday, enabled: smallPayments) }

    var spendEntries: [SpendEntry] {
        history.map { SpendEntry(amountMinor: $0.amountMinor, category: SpendCategory(rawValue: $0.category ?? "") ?? .other, date: $0.date, paid: $0.status == .success) }
    }

    var historyRows: [HistoryRow] {
        history.map { HistoryRow(date: $0.date, name: $0.name, payee: $0.payee, amountMinor: $0.amountMinor, paid: $0.status == .success,
                                 category: SpendCategory(rawValue: $0.category ?? "") ?? .other, note: $0.note, kind: $0.kind ?? "",
                                 tags: vault.entry($0.id.uuidString).tags) }
    }
    var budgetStatuses: [BudgetStatus] { BudgetCheck.statuses(spendEntries, budgets: budgets, now: Date()) }

    /// A test transfer to a person. The money moves between **test wallets on this phone**; Vayora's server and ledger are not involved.
    func transfer(_ p: UPIPayload, amount: Money) -> Outcome {
        guard amount.minor > 0 else { return .declined("Enter an amount") }
        guard amount.minor <= balanceMinor else { return .declined("Not enough test balance") }
        return .paid
    }

    /// Records a finished payment: history, balance, the payee book and a category guess.
    func settle(_ p: UPIPayload, amount: Money, outcome: Outcome, transfer: Bool, noPIN: Bool, viaCircle: CircleRecord? = nil) {
        // an unclear outcome (checking, server away) is never recorded: nothing is known yet
        let ok: Bool
        switch outcome { case .paid: ok = true; case .declined: ok = false; case .unknown, .notAvailable: return }
        let cat = categoryMemory.resolve(vpa: p.payee, name: p.name, note: p.note, transfer: transfer)
        record(Txn(payee: p.payee, name: p.name, amountMinor: amount.minor, note: p.note, status: ok ? .success : .failed,
                   kind: viaCircle != nil ? "circle-out" : (transfer ? "transfer" : nil), category: cat.rawValue, noPIN: noPIN ? true : nil))
        if ok { payees.save(name: p.name, input: p.payee, paidAt: Date()) }
    }

    private func save() {
        if let d = try? JSONEncoder().encode(history.prefix(200).map { $0 }) { UserDefaults.standard.set(d, forKey: "history") }
    }

    /// Built-in demo bank: no server. Stored per phone.
    @Published var demoMode: Bool = UserDefaults.standard.bool(forKey: "demoMode") { didSet { UserDefaults.standard.set(demoMode, forKey: "demoMode") } }

    var client: VayoraClient? { demoMode ? .demo() : URL(string: serverURL).map { VayoraClient(baseURL: $0) } }
    var balance: Money { Money(minor: balanceMinor) }

    var spentThisMonth: Int64 {
        let cal = Calendar.current
        return history.filter { $0.status == .success && cal.isDate($0.date, equalTo: Date(), toGranularity: .month) }
            .reduce(0) { $0 + $1.amountMinor }
    }

    func record(_ t: Txn) {
        history.insert(t, at: 0)
        // a payment made through someone else's circle comes out of their account, not this wallet
        if t.status == .success && t.kind != "circle-out" { balanceMinor -= t.amountMinor }
    }

    func resetWallet() { balanceMinor = Store.startingBalance; history = []; jars = JarBook() }

    /// The person says what a payee is: this and every earlier payment to it, and later ones, use that category.
    func setCategory(_ c: SpendCategory, forPayee vpa: String) {
        categoryMemory.set(vpa, to: c)
        for i in history.indices where history[i].payee.lowercased() == vpa.lowercased() { history[i].category = c.rawValue }
    }

    /// Wallet to jar and back. Failures leave both untouched.
    func moveToJar(_ id: UUID, _ minor: Int64) -> JarBook.Failure? {
        var w = balanceMinor, b = jars
        do { try b.deposit(id, minor, wallet: &w) } catch { return error as? JarBook.Failure }
        balanceMinor = w; jars = b; return nil
    }
    func moveFromJar(_ id: UUID, _ minor: Int64) -> JarBook.Failure? {
        var w = balanceMinor, b = jars
        do { try b.withdraw(id, minor, wallet: &w) } catch { return error as? JarBook.Failure }
        balanceMinor = w; jars = b; return nil
    }
    func removeJar(_ id: UUID) { var w = balanceMinor, b = jars; b.remove(id, wallet: &w); balanceMinor = w; jars = b }

    /// Pay a scanned payload through the simulated payer. Returns the final state; a lost reply is
    /// reported as `.unknown` so the screen says "checking" instead of guessing.
    enum Outcome: Equatable { case paid, declined(String), unknown, notAvailable }

    func pay(_ p: UPIPayload, amount: Money, circle: CircleRecord? = nil) async -> Outcome {
        if circle == nil { guard amount.minor <= balanceMinor else { return .declined("Not enough test balance") } }
        guard let c = client else { return .notAvailable }
        do {
            let uri = rebuildURI(p, amount: amount)
            let status: String
            if let circle {
                let r = try await c.simCirclePay(circleID: circle.id, upiURI: uri)
                if let i = circles.firstIndex(where: { $0.id == circle.id }) { circles[i].remainingMinor = r.remainingMinor }
                status = r.status
            } else { status = try await c.simPay(upiURI: uri).status }
            return status == "captured" ? .paid : .declined("Payment was not completed")
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

    /// A setup QR (`vayora://setup`) sets the test server. Returns the host when it was one, so any scanner can accept it.
    func applySetup(_ text: String) -> String? {
        guard let p = try? SetupPayload.parse(text) else { return nil }
        serverURL = p.server.absoluteString
        return p.server.host ?? "server"
    }

    // MARK: AutoPay

    /// The payer approves a mandate in the simulated bank, and this phone remembers it. A failure leaves nothing saved.
    func approveMandate(_ m: MandatePayload, uri: String) async -> Result<Void, APIError> {
        guard let c = client else { return .failure(APIError(status: 0, code: "no_server", message: "Set the test server in Settings first.")) }
        do {
            let r = try await c.simMandateDecide(upiURI: uri, approve: true)
            guard r.status == "active" else { return .failure(APIError(status: 0, code: "not_active", message: "The bank did not turn it on (\(r.status)).")) }
            let until = m.end.map { Self.isoDay($0) }
            autoPays.removeAll { $0.id == m.mandateID }
            autoPays.insert(ApprovedMandate(id: m.mandateID, merchant: m.name, payee: m.payee, capMinor: m.cap.minor, frequency: m.frequency.rawValue, until: until, status: "active"), at: 0)
            payees.save(name: m.name, input: m.payee)
            return .success(())
        } catch let e as APIError { return .failure(e) }
        catch { return .failure(APIError(status: 0, code: "network", message: "Couldn't reach the test server.")) }
    }

    /// Ask the bank about every AutoPay: its state, and the debits it took. New debits leave the test wallet **once**
    /// (by attempt id) and appear in History. Failures are quiet: the next poll tries again.
    func refreshAutoPays() async {
        guard let c = client else { return }
        for m in autoPays {
            guard let v = try? await c.simMandateView(id: m.id), let i = autoPays.firstIndex(where: { $0.id == m.id }) else { continue }
            autoPays[i].status = v.status
            let fresh = AutoPayLedger.newDebits(applied: Set(autoPays[i].debits.map(\.attemptID)), reported: v.debits)
            for d in fresh {
                balanceMinor = max(0, balanceMinor - d.amountMinor)
                history.insert(Txn(payee: m.payee, name: m.merchant, amountMinor: d.amountMinor, note: "AutoPay", status: .success, kind: "autopay", category: SpendCategory.guess(name: m.merchant, note: "", transfer: false).rawValue), at: 0)
            }
            autoPays[i].debits.append(contentsOf: fresh)
        }
    }

    /// The payer stops an AutoPay from their own app.
    func stopAutoPay(_ id: String) async -> Bool {
        guard let c = client, (try? await c.simMandateRevoke(id: id)) != nil, let i = autoPays.firstIndex(where: { $0.id == id }) else { return false }
        autoPays[i].status = "revoked"
        return true
    }

    static func isoDay(_ d: Date) -> String {
        var cal = Calendar(identifier: .gregorian); cal.timeZone = .current
        let c = cal.dateComponents([.year, .month, .day], from: d)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    // MARK: Backup

    private static let backupDataKeys = ["payees", "history", "reminders", "scheduled", "splits", "budgets", "categoryMemory", "jars", "vault", "autoPays"]

    /// The phone's own data as a file: no PIN, no server address, no photos (they stay on the phone).
    func exportBackup() -> Data? {
        let d = UserDefaults.standard
        var v: [String: Any] = [:]
        for k in Self.backupDataKeys { if let data = d.data(forKey: k), let o = try? JSONSerialization.jsonObject(with: data) { v[k] = o } }
        v["myName"] = myName; v["smallPayments"] = smallPayments; v["balance"] = balanceMinor
        return try? Backup.make(v)
    }

    /// Replace this phone's data with a backup. Everything is checked first; a value that does not decode is skipped, not half-applied.
    func restoreBackup(_ file: Data) -> Result<Int, Backup.Failure> {
        let v: [String: Any]
        do { v = try Backup.read(file) } catch { return .failure(error as? Backup.Failure ?? .notBackup) }
        func decode<T: Decodable>(_ k: String, _ t: T.Type) -> T? {
            guard let o = v[k], JSONSerialization.isValidJSONObject(o), let d = try? JSONSerialization.data(withJSONObject: o) else { return nil }
            return try? JSONDecoder().decode(T.self, from: d)
        }
        var n = 0
        if let x = decode("payees", PayeeBook.self) { payees = x; n += 1 }
        if let x = decode("history", [Txn].self) { history = x; n += 1 }
        if let x = decode("reminders", [BillReminder].self) { reminders = x; n += 1 }
        if let x = decode("scheduled", [ScheduledPayment].self) { scheduled = x; n += 1 }
        if let x = decode("splits", [SplitRecord].self) { splits = x; n += 1 }
        if let x = decode("budgets", Budgets.self) { budgets = x; n += 1 }
        if let x = decode("categoryMemory", CategoryMemory.self) { categoryMemory = x; n += 1 }
        if let x = decode("jars", JarBook.self) { jars = x; n += 1 }
        if let x = decode("vault", Vault.self) { vault = x; n += 1 }
        if let x = decode("autoPays", [ApprovedMandate].self) { autoPays = x; n += 1 }
        if let x = v["myName"] as? String { myName = x; n += 1 }
        if let x = v["smallPayments"] as? Bool { smallPayments = x; n += 1 }
        if let x = (v["balance"] as? NSNumber)?.int64Value { balanceMinor = x; n += 1 }
        return .success(n)
    }

    // MARK: Circle (test mode)

    private var meForCircle: String { myName.trimmingCharacters(in: .whitespaces).isEmpty ? "Me" : myName }

    /// Let someone pay from this phone's account, up to a limit. The simulated bank keeps the books.
    func createCircle(delegate: String, perPayment: Money, total: Money) async -> Result<CircleRecord, APIError> {
        guard let c = client else { return .failure(APIError(status: 0, code: "no_server", message: "Set the test server in Settings first.")) }
        do {
            let id = try await c.simCircleCreate(primary: meForCircle, delegate: delegate, perPayment: perPayment, total: total)
            let r = CircleRecord(id: id, role: .primary, other: delegate, perPaymentMinor: perPayment.minor, totalMinor: total.minor)
            circles.insert(r, at: 0); return .success(r)
        } catch let e as APIError { return .failure(e) }
        catch { return .failure(APIError(status: 0, code: "network", message: "Couldn't reach the test server.")) }
    }

    /// Join a circle from its QR. Returns a message when it cannot be joined.
    func joinCircle(_ link: CirclePayload.Link) async -> String? {
        guard let c = client else { return "Set the test server in Settings first." }
        if circles.contains(where: { $0.id == link.id }) { return "You are already part of that circle." }
        do {
            let v = try await c.simCircleView(id: link.id)
            guard v.status == "active" else { return "That circle has been taken back." }
            var r = CircleRecord(id: link.id, role: .delegate, other: v.primary, perPaymentMinor: v.maxPerPaymentMinor, totalMinor: v.totalLimitMinor)
            r.remainingMinor = v.remainingMinor
            circles.insert(r, at: 0); return nil
        } catch let e as APIError { return e.status == 404 ? "That circle isn't known to this test server." : e.message }
        catch { return "Couldn't reach the test server." }
    }

    /// Ask the bank about every circle. As the lender, take new spends out of this wallet once each (floored at zero) and list them in History.
    func refreshCircles() async {
        guard let c = client else { return }
        for r in circles {
            guard let v = try? await c.simCircleView(id: r.id), let i = circles.firstIndex(where: { $0.id == r.id }) else { continue }
            circles[i].status = v.status; circles[i].remainingMinor = v.remainingMinor
            guard r.role == .primary else { continue }
            let fresh = AutoPayLedger.newDebits(applied: Set(circles[i].applied.map(\.attemptID)), reported: v.spends)
            for d in fresh {
                balanceMinor = max(0, balanceMinor - d.amountMinor)
                history.insert(Txn(payee: "circle", name: "\(r.other) (circle)", amountMinor: d.amountMinor, note: "Paid by \(r.other) from your account", status: .success, kind: "circle-in", category: SpendCategory.other.rawValue), at: 0)
            }
            circles[i].applied.append(contentsOf: fresh)
        }
    }

    /// The lender takes the circle back; a delegate just leaves it on this phone.
    func endCircle(_ id: String) async -> Bool {
        guard let i = circles.firstIndex(where: { $0.id == id }) else { return false }
        if circles[i].role == .primary {
            guard let c = client, (try? await c.simCircleRevoke(id: id)) != nil else { return false }
            circles[i].status = "revoked"
        } else { circles.remove(at: i) }
        return true
    }
}
