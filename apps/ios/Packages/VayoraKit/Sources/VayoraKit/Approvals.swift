import Foundation

/// A refund or payout that is waiting for a second person's yes.
public struct PendingApproval: Equatable, Hashable, Sendable, Identifiable {
    public enum Kind: String, Sendable { case refund, payout }
    public let kind: Kind
    public let rawID: String
    public let amountMinor: Int64
    public let currency: String
    public let detail: String
    public let requestedBy: String
    public var id: String { kind.rawValue + ":" + rawID }
    public var title: String { kind == .refund ? "Refund" : "Payout" }
    public init(kind: Kind, rawID: String, amountMinor: Int64, currency: String = "INR", detail: String = "", requestedBy: String = "") {
        self.kind = kind; self.rawID = rawID; self.amountMinor = amountMinor; self.currency = currency; self.detail = detail; self.requestedBy = requestedBy
    }
}

struct RefundRowDTO: Decodable {
    let id: String; let amountMinor: Int64; let currency: String; let status: String; let reason: String?; let requestedBy: String
    enum CodingKeys: String, CodingKey { case id, currency, status, reason, amountMinor = "amount_minor", requestedBy = "requested_by" }
}
struct PayoutRowDTO: Decodable {
    let id: String; let amountMinor: Int64; let currency: String; let status: String; let purpose: String?; let requestedBy: String
    enum CodingKeys: String, CodingKey { case id, currency, status, purpose, amountMinor = "amount_minor", requestedBy = "requested_by" }
}

extension VayoraClient {
    /// Refunds and payouts still waiting for approval, newest kind first within each list as the server returns them.
    public func pendingApprovals() async throws -> [PendingApproval] {
        struct Refunds: Decodable { let data: [RefundRowDTO] }
        struct Payouts: Decodable { let data: [PayoutRowDTO] }
        async let r = send(request("GET", "/refunds?status=pending_approval"), as: Refunds.self)
        async let p = send(request("GET", "/payouts?status=pending_approval"), as: Payouts.self)
        let (refunds, payouts) = try await (r, p)
        return refunds.data.filter { $0.status == "pending_approval" }.map {
            PendingApproval(kind: .refund, rawID: $0.id, amountMinor: $0.amountMinor, currency: $0.currency, detail: $0.reason ?? "", requestedBy: $0.requestedBy)
        } + payouts.data.filter { $0.status == "pending_approval" }.map {
            PendingApproval(kind: .payout, rawID: $0.id, amountMinor: $0.amountMinor, currency: $0.currency, detail: $0.purpose ?? "", requestedBy: $0.requestedBy)
        }
    }

    /// Approves or rejects. The server refuses the person who made the request (separation of duties, a 403) and
    /// anyone without the approve permission; that message is shown as it is.
    public func decide(_ a: PendingApproval, approve: Bool) async throws {
        struct Done: Decodable {}
        let base = a.kind == .refund ? "/refunds" : "/payouts"
        _ = try await send(request("POST", "\(base)/\(a.rawID)/\(approve ? "approve" : "reject")"), as: Done.self)
    }
}
