import Foundation

/// A debit the payer's bank took under a mandate.
public struct PayerDebit: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let attemptID: String
    public let amountMinor: Int64
    public let at: String
    public var id: String { attemptID }
    public var money: Money { Money(minor: amountMinor) }
    enum CodingKeys: String, CodingKey { case attemptID = "attempt_id", amountMinor = "amount_minor", at }
    public init(attemptID: String, amountMinor: Int64, at: String) { self.attemptID = attemptID; self.amountMinor = amountMinor; self.at = at }
}

public enum AutoPayLedger {
    /// The debits in `reported` that have not been applied yet, in order and without repeats. Polling the payer's
    /// view twice, or two screens polling at once, never takes the same debit from the wallet twice.
    public static func newDebits(applied: Set<String>, reported: [PayerDebit]) -> [PayerDebit] {
        var seen = applied
        return reported.filter { seen.insert($0.attemptID).inserted }
    }

    public static func total(_ debits: [PayerDebit]) -> Money { Money(minor: debits.reduce(0) { $0 + $1.amountMinor }) }
}
