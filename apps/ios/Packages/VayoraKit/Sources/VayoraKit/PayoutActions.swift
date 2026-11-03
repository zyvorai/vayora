import Foundation

/// A payout that has been approved and is waiting for the transfer to be made and recorded.
public struct ApprovedPayout: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let amountMinor: Int64
    public let currency: String
    public let purpose: String?
    enum CodingKeys: String, CodingKey { case id, currency, purpose, amountMinor = "amount_minor" }
    public init(id: String, amountMinor: Int64, currency: String = "INR", purpose: String? = nil) { self.id = id; self.amountMinor = amountMinor; self.currency = currency; self.purpose = purpose }
}

/// A refund the provider could not complete.
public struct FailedRefund: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let amountMinor: Int64
    public let currency: String
    public let failureReason: String?
    enum CodingKeys: String, CodingKey { case id, currency, amountMinor = "amount_minor", failureReason = "failure_reason" }
    public init(id: String, amountMinor: Int64, currency: String = "INR", failureReason: String? = nil) { self.id = id; self.amountMinor = amountMinor; self.currency = currency; self.failureReason = failureReason }
}

/// The bank's own reference for a transfer (the UTR). The server wants 6 to 40 characters after trimming.
public enum BankRef {
    public static func cleaned(_ s: String) -> String { s.trimmingCharacters(in: .whitespacesAndNewlines) }
    public static func isValid(_ s: String) -> Bool { (6...40).contains(cleaned(s).count) }
}

extension VayoraClient {
    public func approvedPayouts() async throws -> [ApprovedPayout] {
        struct Row: Decodable { let id: String; let amountMinor: Int64; let currency: String; let purpose: String?; let status: String
            enum CodingKeys: String, CodingKey { case id, currency, purpose, status, amountMinor = "amount_minor" } }
        struct List: Decodable { let data: [Row] }
        return try await send(request("GET", "/payouts?status=approved"), as: List.self).data.filter { $0.status == "approved" }
            .map { ApprovedPayout(id: $0.id, amountMinor: $0.amountMinor, currency: $0.currency, purpose: $0.purpose) }
    }

    /// Records that the transfer was made (once; it posts to the ledger). The caller has the person confirm first.
    public func markPayoutPaid(id: String, bankRef: String) async throws {
        struct Done: Decodable {}
        let data = try JSONSerialization.data(withJSONObject: ["bank_ref": BankRef.cleaned(bankRef)])
        _ = try await send(request("POST", "/payouts/\(id)/mark_paid", body: data), as: Done.self)
    }

    public func failedRefunds() async throws -> [FailedRefund] {
        struct Row: Decodable { let id: String; let amountMinor: Int64; let currency: String; let status: String; let failureReason: String?
            enum CodingKeys: String, CodingKey { case id, currency, status, amountMinor = "amount_minor", failureReason = "failure_reason" } }
        struct List: Decodable { let data: [Row] }
        return try await send(request("GET", "/refunds?status=failed"), as: List.self).data.filter { $0.status == "failed" }
            .map { FailedRefund(id: $0.id, amountMinor: $0.amountMinor, currency: $0.currency, failureReason: $0.failureReason) }
    }

    public func retryRefund(id: String) async throws {
        struct Done: Decodable {}
        _ = try await send(request("POST", "/refunds/\(id)/retry"), as: Done.self)
    }
}
