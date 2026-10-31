import Foundation

public struct APIError: Error, Equatable, Sendable, LocalizedError {
    public let status: Int
    public let code: String
    public let message: String
    public var errorDescription: String? { message }
}

public enum Attempt: String, Codable, Sendable {
    case created, pending, authorized, captured, failed, unknown, canceled
}

public struct PaymentIntent: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let amountMinor: Int64
    public let currency: String
    public let status: String
    public let clientSecret: String?
    public let createdAt: String?

    enum CodingKeys: String, CodingKey {
        case id, currency, status
        case amountMinor = "amount_minor"
        case clientSecret = "client_secret"
        case createdAt = "created_at"
    }
    public var money: Money { Money(minor: amountMinor, currency: currency) }
}

public struct ClientActionInfo: Codable, Equatable, Sendable {
    public let type: String
    public let uri: String?
}

public struct AttemptInfo: Codable, Equatable, Sendable {
    public let id: String
    public let status: String
    public let failureReason: String?
    public let clientAction: ClientActionInfo?
    public let amountMinor: Int64?
    public let refundedMinor: Int64?
    public let reservedRefundMinor: Int64?
    enum CodingKeys: String, CodingKey {
        case id, status
        case failureReason = "failure_reason"
        case clientAction = "client_action"
        case amountMinor = "amount_minor"
        case refundedMinor = "refunded_minor"
        case reservedRefundMinor = "reserved_refund_minor"
    }

    /// The `upi://` payload to show as a QR, when the attempt issued one.
    public var upiURI: String? {
        guard let a = clientAction, a.type == "upi_intent", let u = a.uri, u.lowercased().hasPrefix("upi://") else { return nil }
        return u
    }
    public var isPaid: Bool { status == "captured" }
    public var isFinal: Bool { ["captured", "failed", "canceled"].contains(status) }
}

struct ErrorEnvelope: Decodable {
    struct Body: Decodable { let code: String; let message: String }
    let error: Body
}

/// How much of a captured payment has been (or is being) given back. A refund in progress counts as given back,
/// so the app never offers to refund money that is already on its way.
public struct RefundState: Equatable, Sendable {
    public let capturedMinor: Int64
    public let refundedMinor: Int64
    public init(capturedMinor: Int64, refundedMinor: Int64) { self.capturedMinor = capturedMinor; self.refundedMinor = refundedMinor }
    public var remainingMinor: Int64 { max(0, capturedMinor - refundedMinor) }
    public var isFullyRefunded: Bool { capturedMinor > 0 && remainingMinor == 0 }
    public var isPartlyRefunded: Bool { refundedMinor > 0 && remainingMinor > 0 }

    public init?(attempts: [AttemptInfo]) {
        guard let a = attempts.first(where: { $0.status == "captured" }), let total = a.amountMinor else { return nil }
        self.init(capturedMinor: total, refundedMinor: (a.refundedMinor ?? 0) + (a.reservedRefundMinor ?? 0))
    }
}

public struct RefundInfo: Codable, Equatable, Sendable {
    public let id: String
    public let status: String
    public let amountMinor: Int64
    enum CodingKeys: String, CodingKey { case id, status; case amountMinor = "amount_minor" }
}

public struct SimPayResult: Codable, Equatable, Sendable { public let status: String }

/// Talks to a Vayora API. The transport is injectable so tests never touch the network.
public struct VayoraClient: Sendable {
    public var baseURL: URL
    public var apiKey: String?
    public var session: URLSession

    public init(baseURL: URL, apiKey: String? = nil, session: URLSession = .shared) {
        self.baseURL = baseURL; self.apiKey = apiKey; self.session = session
    }

    func request(_ method: String, _ path: String, body: Data? = nil, idempotencyKey: String? = nil) -> URLRequest {
        var r = URLRequest(url: baseURL.appendingPathComponent("v1" + path))
        r.httpMethod = method
        r.timeoutInterval = 15
        r.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let k = apiKey { r.setValue("Bearer \(k)", forHTTPHeaderField: "Authorization") }
        if let i = idempotencyKey { r.setValue(i, forHTTPHeaderField: "Idempotency-Key") }
        r.httpBody = body
        return r
    }

    func send<T: Decodable>(_ r: URLRequest, as: T.Type) async throws -> T {
        let (data, resp) = try await session.data(for: r)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            let e = try? JSONDecoder().decode(ErrorEnvelope.self, from: data)
            throw APIError(status: status, code: e?.error.code ?? "http_\(status)", message: e?.error.message ?? "Request failed (\(status))")
        }
        return try JSONDecoder().decode(T.self, from: data)
    }

    // Merchant

    public func createIntent(amount: Money, note: String?, idempotencyKey: String = UUID().uuidString) async throws -> PaymentIntent {
        var body: [String: Any] = ["amount_minor": amount.minor, "currency": amount.currency]
        if let note, !note.isEmpty { body["description"] = note }
        let data = try JSONSerialization.data(withJSONObject: body)
        return try await send(request("POST", "/payment_intents", body: data, idempotencyKey: idempotencyKey), as: PaymentIntent.self)
    }

    public func intent(id: String) async throws -> PaymentIntent {
        struct Detail: Decodable { let intent: PaymentIntent }
        return try await send(request("GET", "/payment_intents/\(id)"), as: Detail.self).intent
    }

    /// Asks the server to re-check every open attempt of a payment with its provider, then returns the payment as it
    /// now stands. A payment is only recorded as paid once someone asks, so lists call this for unfinished payments.
    public func settle(id: String) async throws -> PaymentIntent {
        struct Detail: Decodable { let intent: PaymentIntent; let attempts: [AttemptInfo] }
        let d = try await send(request("GET", "/payment_intents/\(id)"), as: Detail.self)
        var asked = false
        for a in d.attempts where a.status == "pending" || a.status == "authorized" {
            _ = try? await refreshAttempt(id: a.id)
            asked = true
        }
        return asked ? try await intent(id: id) : d.intent
    }

    /// Refund position of a payment, or nil when it has no captured attempt.
    public func refundState(id: String) async throws -> RefundState? {
        struct Detail: Decodable { let attempts: [AttemptInfo] }
        let d = try await send(request("GET", "/payment_intents/\(id)"), as: Detail.self)
        return RefundState(attempts: d.attempts)
    }

    public func listIntents() async throws -> [PaymentIntent] {
        struct List: Decodable { let data: [PaymentIntent] }
        return try await send(request("GET", "/payment_intents"), as: List.self).data
    }

    /// Asks for a UPI QR / intent payload for the intent (the partner's, or the simulator's in test).
    public func createUPIAttempt(intentId: String, idempotencyKey: String = UUID().uuidString) async throws -> AttemptInfo {
        let data = try JSONSerialization.data(withJSONObject: ["method": "upi", "upi_flow": "intent"])
        return try await send(request("POST", "/payment_intents/\(intentId)/attempts", body: data, idempotencyKey: idempotencyKey), as: AttemptInfo.self)
    }

    public func refund(intentId: String, amount: Money?, reason: String?, idempotencyKey: String = UUID().uuidString) async throws -> RefundInfo {
        var body: [String: Any] = [:]
        if let amount { body["amount_minor"] = amount.minor }
        if let reason, !reason.isEmpty { body["reason"] = reason }
        let data = try JSONSerialization.data(withJSONObject: body)
        return try await send(request("POST", "/payment_intents/\(intentId)/refunds", body: data, idempotencyKey: idempotencyKey), as: RefundInfo.self)
    }

    public func refreshAttempt(id: String) async throws -> AttemptInfo {
        try await send(request("POST", "/payment_attempts/\(id)/refresh"), as: AttemptInfo.self)
    }

    // Payer (simulator only; the server answers 404 unless ZYVOR_SIM_PAYER=1)

    public func simPay(upiURI: String, approve: Bool = true) async throws -> SimPayResult {
        let data = try JSONSerialization.data(withJSONObject: ["upi_uri": upiURI, "approve": approve])
        return try await send(request("POST", "/sim/pay", body: data), as: SimPayResult.self)
    }
}
