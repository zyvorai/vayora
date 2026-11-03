import Foundation

/// A payment link as the server lists it. The `path` is relative to the public checkout address, which the phone is told in Settings.
public struct PaymentLinkInfo: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let path: String
    public let description: String?
    public let amountMinor: Int64?
    public let currency: String
    public let invoiceID: String?
    public let maxUses: Int?
    public let uses: Int
    public let expiresAt: String?
    public let status: String
    public let createdAt: String?
    public let canEmail: Bool

    enum CodingKeys: String, CodingKey {
        case id, path, description, currency, uses, status
        case amountMinor = "amount_minor", invoiceID = "invoice_id", maxUses = "max_uses"
        case expiresAt = "expires_at", createdAt = "created_at", canEmail = "can_email"
    }

    public init(id: String, path: String, description: String? = nil, amountMinor: Int64? = nil, currency: String = "INR", invoiceID: String? = nil,
                maxUses: Int? = nil, uses: Int = 0, expiresAt: String? = nil, status: String = "active", createdAt: String? = nil, canEmail: Bool = false) {
        self.id = id; self.path = path; self.description = description; self.amountMinor = amountMinor; self.currency = currency; self.invoiceID = invoiceID
        self.maxUses = maxUses; self.uses = uses; self.expiresAt = expiresAt; self.status = status; self.createdAt = createdAt; self.canEmail = canEmail
    }

    public var isActive: Bool { status == "active" }

    /// The address to share: the checkout address plus the link's path. Nil unless the checkout address is a plain https address
    /// (a link is money, so it is never built from anything else) and the path is a `/pay/link/…` path.
    public func shareURL(checkoutBase: String) -> URL? {
        let base = checkoutBase.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var c = URLComponents(string: base), c.scheme == "https", let host = c.host, !host.isEmpty,
              c.user == nil, c.password == nil, c.query == nil, c.fragment == nil,
              path.hasPrefix("/pay/link/"), !path.contains("..") else { return nil }
        let prefix = c.path.hasSuffix("/") ? String(c.path.dropLast()) : c.path
        c.path = prefix + path
        return c.url
    }
}

public struct CreatedPaymentLink: Codable, Equatable, Sendable { public let id: String; public let path: String }

extension VayoraClient {
    public func paymentLinks() async throws -> [PaymentLinkInfo] {
        struct List: Decodable { let data: [PaymentLinkInfo] }
        return try await send(request("GET", "/payment_links"), as: List.self).data
    }

    /// A fixed-amount link. `expiresAt` is an ISO-8601 time; `maxUses` stops it after that many checkouts.
    public func createPaymentLink(amount: Money, description: String?, maxUses: Int? = nil, expiresAt: Date? = nil, idempotencyKey: String = UUID().uuidString) async throws -> CreatedPaymentLink {
        var body: [String: Any] = ["amount_minor": amount.minor, "currency": amount.currency]
        if let description, !description.isEmpty { body["description"] = description }
        if let maxUses { body["max_uses"] = maxUses }
        if let expiresAt { body["expires_at"] = ISO8601DateFormatter().string(from: expiresAt) }
        let data = try JSONSerialization.data(withJSONObject: body)
        return try await send(request("POST", "/payment_links", body: data, idempotencyKey: idempotencyKey), as: CreatedPaymentLink.self)
    }

    public func cancelPaymentLink(id: String) async throws {
        struct Done: Decodable { let status: String }
        _ = try await send(request("POST", "/payment_links/\(id)/cancel"), as: Done.self)
    }

    /// Emails the link to its customer (the server refuses a link with no customer email, or when it has no checkout address).
    public func emailPaymentLink(id: String) async throws {
        struct Done: Decodable {}
        _ = try await send(request("POST", "/payment_links/\(id)/send"), as: Done.self)
    }
}
