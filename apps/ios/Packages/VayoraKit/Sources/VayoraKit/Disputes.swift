import Foundation

/// A chargeback-style dispute on a payment.
public struct DisputeInfo: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let amountMinor: Int64
    public let reason: String
    public let status: String
    public let respondBy: String?
    public let evidenceCount: Int
    enum CodingKeys: String, CodingKey { case id, reason, status, amountMinor = "amount_minor", respondBy = "respond_by", evidenceCount = "evidence_count" }
    public init(id: String, amountMinor: Int64, reason: String, status: String = "open", respondBy: String? = nil, evidenceCount: Int = 0) {
        self.id = id; self.amountMinor = amountMinor; self.reason = reason; self.status = status; self.respondBy = respondBy; self.evidenceCount = evidenceCount
    }
    /// Still waiting on the merchant or the issuer.
    public var isOpen: Bool { status == "open" || status == "under_review" }

    /// Whole days from `today` (yyyy-MM-dd) to the reply deadline; negative once it has passed. Nil without a deadline.
    public func daysLeft(today: String) -> Int? {
        guard let respondBy else { return nil }
        let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd"; f.locale = Locale(identifier: "en_US_POSIX"); f.timeZone = TimeZone(identifier: "Asia/Kolkata")
        guard let a = f.date(from: today), let b = f.date(from: respondBy) else { return nil }
        var cal = Calendar(identifier: .gregorian); cal.timeZone = f.timeZone
        return cal.dateComponents([.day], from: a, to: b).day
    }
}

extension VayoraClient {
    /// Disputes that still need an answer, soonest deadline first (the server's order).
    public func openDisputes() async throws -> [DisputeInfo] {
        struct List: Decodable { let data: [DisputeInfo] }
        return try await send(request("GET", "/disputes"), as: List.self).data.filter(\.isOpen)
    }

    /// Adds a written note, and optionally where the document lives. Zyvor keeps the reference, not a file.
    public func addDisputeEvidence(id: String, note: String, url: String? = nil) async throws {
        struct Done: Decodable {}
        var body: [String: Any] = ["note": note]
        if let url, !url.isEmpty { body["url"] = url }
        let data = try JSONSerialization.data(withJSONObject: body)
        _ = try await send(request("POST", "/disputes/\(id)/evidence", body: data), as: Done.self)
    }

    /// Records the issuer's decision. Losing books the loss on the ledger and re-opens the invoice, so callers confirm first.
    public func resolveDispute(id: String, won: Bool) async throws {
        struct Done: Decodable {}
        let data = try JSONSerialization.data(withJSONObject: ["outcome": won ? "won" : "lost"])
        _ = try await send(request("POST", "/disputes/\(id)/resolve", body: data), as: Done.self)
    }
}
