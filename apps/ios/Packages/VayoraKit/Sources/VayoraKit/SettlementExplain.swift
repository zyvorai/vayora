import Foundation

/// Why a settlement paid what it did: the provider's gross, what it took off, what was expected, and the gap.
public struct SettlementExplanation: Decodable, Equatable, Sendable {
    public struct Reason: Decodable, Equatable, Sendable, Identifiable {
        public let reason: String
        public let amountMinor: Int64
        public var id: String { reason }
        enum CodingKeys: String, CodingKey { case reason, amountMinor = "amount_minor" }
        public init(reason: String, amountMinor: Int64) { self.reason = reason; self.amountMinor = amountMinor }
    }
    public struct Case: Decodable, Equatable, Sendable, Identifiable {
        public let id: String
        public let kind: String
        public let status: String
        public let explanation: String?
    }
    public let narrative: String
    public let expectedMinor: Int64
    public let receivedMinor: Int64
    public let differenceMinor: Int64
    public let breakdown: [Reason]
    public let balances: Bool
    public let paymentsMatched: Int
    public let paymentsUnmatched: Int
    public let matchedBank: Bool
    public let cases: [Case]

    enum K: String, CodingKey { case narrative, payments, cases, arithmetic, expectedMinor = "expected_minor", receivedMinor = "received_minor", differenceMinor = "difference_minor", breakdown = "difference_breakdown", bankMatch = "bank_match" }
    struct Arithmetic: Decodable { let balances: Bool }
    struct Pay: Decodable { let matched: Bool }
    struct AnyValue: Decodable { init(from d: Decoder) throws {} }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: K.self)
        narrative = try c.decode(String.self, forKey: .narrative)
        expectedMinor = try c.decode(Int64.self, forKey: .expectedMinor)
        receivedMinor = try c.decode(Int64.self, forKey: .receivedMinor)
        differenceMinor = try c.decode(Int64.self, forKey: .differenceMinor)
        breakdown = try c.decodeIfPresent([Reason].self, forKey: .breakdown) ?? []
        balances = try c.decodeIfPresent(Arithmetic.self, forKey: .arithmetic)?.balances ?? true
        let pays = try c.decodeIfPresent([Pay].self, forKey: .payments) ?? []
        paymentsMatched = pays.filter(\.matched).count; paymentsUnmatched = pays.filter { !$0.matched }.count
        matchedBank = ((try? c.decodeIfPresent(AnyValue.self, forKey: .bankMatch)) ?? nil) != nil   // null or missing means no bank line yet
        cases = try c.decodeIfPresent([Case].self, forKey: .cases) ?? []
    }

    public init(narrative: String, expectedMinor: Int64, receivedMinor: Int64, breakdown: [Reason] = [], balances: Bool = true, paymentsMatched: Int = 0, paymentsUnmatched: Int = 0, matchedBank: Bool = false, cases: [Case] = []) {
        self.narrative = narrative; self.expectedMinor = expectedMinor; self.receivedMinor = receivedMinor; self.differenceMinor = receivedMinor - expectedMinor
        self.breakdown = breakdown; self.balances = balances; self.paymentsMatched = paymentsMatched; self.paymentsUnmatched = paymentsUnmatched; self.matchedBank = matchedBank; self.cases = cases
    }

    /// Short and plain: "Matches what was expected" or "₹30.00 less than expected".
    public var verdict: String {
        if differenceMinor == 0 { return "Matches what was expected." }
        let amount = Money(minor: abs(differenceMinor)).display
        return differenceMinor < 0 ? "\(amount) less than expected." : "\(amount) more than expected."
    }
}

extension VayoraClient {
    public func explainSettlement(id: String) async throws -> SettlementExplanation {
        try await send(request("GET", "/settlements/\(id)/explain"), as: SettlementExplanation.self)
    }
}
