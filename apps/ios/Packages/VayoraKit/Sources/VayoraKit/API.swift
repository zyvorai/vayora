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

public struct PaymentIntent: Codable, Equatable, Sendable, Identifiable {
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
    enum CodingKeys: String, CodingKey {
        case id, status
        case failureReason = "failure_reason"
        case clientAction = "client_action"
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
