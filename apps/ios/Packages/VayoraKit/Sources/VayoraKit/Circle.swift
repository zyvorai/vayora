import Foundation

/// The link a primary payer shows so a delegate's phone can join a circle: `vayora://circle?id=cir_…&pn=Asha&per=500.00&total=2000.00`.
/// It carries a name and limits for display; the limits that count are the simulated bank's own, which the delegate's phone asks for.
public enum CirclePayload {
    public enum Failure: Error, Equatable { case notCircle, badID, badName, badAmount, duplicateParameter }

    public struct Link: Equatable, Sendable {
        public let id: String, primary: String, perPayment: Money, total: Money
    }

    public static func make(id: String, primary: String, perPayment: Money, total: Money) -> String {
        var c = URLComponents(); c.scheme = "vayora"; c.host = "circle"
        c.queryItems = [.init(name: "id", value: id), .init(name: "pn", value: primary), .init(name: "per", value: perPayment.decimalString), .init(name: "total", value: total.decimalString)]
        return c.string ?? ""
    }

    public static func parse(_ text: String) throws -> Link {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let c = URLComponents(string: t), c.scheme?.lowercased() == "vayora", c.host?.lowercased() == "circle" else { throw Failure.notCircle }
        let items = c.queryItems ?? []
        func one(_ n: String) throws -> String? {
            let v = items.filter { $0.name == n }
            if v.count > 1 { throw Failure.duplicateParameter }
            return v.first?.value
        }
        guard let id = try one("id"), MandatePayload.isSafeID(id) else { throw Failure.badID }
        let name = (try one("pn") ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, name.count <= 60, !name.contains(where: { $0.isNewline }) else { throw Failure.badName }
        guard let per = try one("per").flatMap({ Money(decimal: $0) }), let total = try one("total").flatMap({ Money(decimal: $0) }), per.minor <= total.minor else { throw Failure.badAmount }
        return Link(id: id, primary: name, perPayment: per, total: total)
    }
}

/// One circle this phone is part of: either it lends its account (`primary`) or it may pay from someone else's (`delegate`).
public struct CircleRecord: Codable, Equatable, Identifiable, Sendable {
    public enum Role: String, Codable, Sendable { case primary, delegate }
    public var id: String
    public var role: Role
    /// The other person's name: who may pay from this phone's account, or whose account this phone may pay from.
    public var other: String
    public var perPaymentMinor: Int64
    public var totalMinor: Int64
    public var status: String = "active"
    public var remainingMinor: Int64
    /// Primary only: the spends already taken from this phone's wallet, so a refresh never takes one twice.
    public var applied: [PayerDebit] = []
    public init(id: String, role: Role, other: String, perPaymentMinor: Int64, totalMinor: Int64) {
        self.id = id; self.role = role; self.other = other; self.perPaymentMinor = perPaymentMinor; self.totalMinor = totalMinor; self.remainingMinor = totalMinor
    }
    public var isLive: Bool { status == "active" }
    /// Whether a delegate could pay this amount now, going by what this phone last heard. The bank decides for real.
    public func canCover(_ amountMinor: Int64) -> Bool { role == .delegate && isLive && amountMinor > 0 && amountMinor <= perPaymentMinor && amountMinor <= remainingMinor }
}

public struct CircleInfo: Codable, Equatable, Sendable {
    public let circleID: String, primary: String, delegate: String, status: String
    public let maxPerPaymentMinor: Int64, totalLimitMinor: Int64, usedMinor: Int64, remainingMinor: Int64
    public let spends: [PayerDebit]
    enum CodingKeys: String, CodingKey {
        case circleID = "circle_id", primary, delegate, status, spends
        case maxPerPaymentMinor = "max_per_payment_minor", totalLimitMinor = "total_limit_minor", usedMinor = "used_minor", remainingMinor = "remaining_minor"
    }
}
