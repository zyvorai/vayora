import Foundation

public struct APIError: Error, Equatable, Sendable, LocalizedError {
    public let status: Int
    public let code: String
    public let message: String
    public var errorDescription: String? { message }
    public init(status: Int, code: String, message: String) { self.status = status; self.code = code; self.message = message }
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

    // Circle (simulator only)

    public func simCircleCreate(primary: String, delegate: String, perPayment: Money, total: Money) async throws -> String {
        struct Created: Decodable { let circle_id: String }
        let body: [String: Any] = ["primary": primary, "delegate": delegate, "max_per_payment_minor": perPayment.minor, "total_limit_minor": total.minor]
        return try await send(request("POST", "/sim/circle", body: try JSONSerialization.data(withJSONObject: body)), as: Created.self).circle_id
    }
    public func simCircleView(id: String) async throws -> CircleInfo {
        guard MandatePayload.isSafeID(id) else { throw APIError(status: 0, code: "bad_id", message: "Not a circle id") }
        return try await send(request("GET", "/sim/circle/\(id)"), as: CircleInfo.self)
    }
    public func simCircleRevoke(id: String) async throws {
        let data = try JSONSerialization.data(withJSONObject: ["circle_id": id])
        struct Done: Decodable { let status: String }
        _ = try await send(request("POST", "/sim/circle/revoke", body: data), as: Done.self)
    }
    public struct CirclePayResult: Decodable, Sendable { public let status: String; public let remainingMinor: Int64
        enum CodingKeys: String, CodingKey { case status, remainingMinor = "remaining_minor" } }
    public func simCirclePay(circleID: String, upiURI: String) async throws -> CirclePayResult {
        let data = try JSONSerialization.data(withJSONObject: ["circle_id": circleID, "upi_uri": upiURI])
        return try await send(request("POST", "/sim/circle/pay", body: data), as: CirclePayResult.self)
    }

    // AutoPay: payer side (simulator only)

    public func simMandateDecide(upiURI: String, approve: Bool = true) async throws -> SimPayResult {
        let data = try JSONSerialization.data(withJSONObject: ["upi_uri": upiURI, "approve": approve])
        return try await send(request("POST", "/sim/mandate", body: data), as: SimPayResult.self)
    }

    /// What the payer's own bank shows: the mandate's state and the debits it took. `id` must already be a safe id
    /// (`MandatePayload` guarantees it for a scanned link).
    public func simMandateView(id: String) async throws -> PayerMandateView {
        guard MandatePayload.isSafeID(id) else { throw APIError(status: 0, code: "bad_id", message: "Not a mandate id") }
        return try await send(request("GET", "/sim/mandate/\(id)"), as: PayerMandateView.self)
    }

    public func simMandateRevoke(id: String) async throws -> SimPayResult {
        let data = try JSONSerialization.data(withJSONObject: ["mandate_id": id])
        return try await send(request("POST", "/sim/mandate/revoke", body: data), as: SimPayResult.self)
    }

    // End of day

    public func branches() async throws -> [BranchInfo] {
        struct List: Decodable { let data: [BranchInfo] }
        return try await send(request("GET", "/branches"), as: List.self).data
    }

    /// Needs the admin scope; a test key from setup has it.
    public func createBranch(name: String) async throws -> BranchInfo {
        let data = try JSONSerialization.data(withJSONObject: ["name": name.trimmingCharacters(in: .whitespaces)])
        return try await send(request("POST", "/branches", body: data), as: BranchInfo.self)
    }

    /// `kind`: "sale" (cash taken), "refund" (cash handed back) or "float" (cash put in the till to start).
    public func addCash(branchID: String, kind: String, amount: Money, note: String? = nil, idempotencyKey: String = UUID().uuidString) async throws {
        var body: [String: Any] = ["branch_id": branchID, "kind": kind, "amount_minor": amount.minor]
        if let note, !note.isEmpty { body["note"] = note }
        struct Created: Decodable { let id: String }
        _ = try await send(request("POST", "/cash_entries", body: try JSONSerialization.data(withJSONObject: body), idempotencyKey: idempotencyKey), as: Created.self)
    }

    /// Close a branch day. `day` is the IST date `YYYY-MM-DD`. A second person signs it off on the console.
    public func closeDay(branchID: String, day: String, counted: Money, note: String? = nil) async throws -> ClosingInfo {
        var body: [String: Any] = ["branch_id": branchID, "day": day, "counted_cash_minor": counted.minor]
        if let note, !note.isEmpty { body["note"] = note }
        return try await send(request("POST", "/closings", body: try JSONSerialization.data(withJSONObject: body)), as: ClosingInfo.self)
    }

    // AutoPay: merchant side

    /// The customer with this name, created the first time. Matched by a reference made from the name, so the same
    /// person is never created twice.
    public func customer(named name: String) async throws -> CustomerInfo {
        let ref = "autopay:" + name.trimmingCharacters(in: .whitespaces).lowercased()
        struct List: Decodable { let data: [CustomerInfo] }
        var comps = URLComponents(); comps.queryItems = [URLQueryItem(name: "external_ref", value: ref)]
        let found = try await send(request("GET", "/customers?" + (comps.percentEncodedQuery ?? "")), as: List.self)
        if let c = found.data.first { return c }
        let data = try JSONSerialization.data(withJSONObject: ["name": name.trimmingCharacters(in: .whitespaces), "external_ref": ref])
        return try await send(request("POST", "/customers", body: data), as: CustomerInfo.self)
    }

    public func customers() async throws -> [CustomerInfo] {
        struct List: Decodable { let data: [CustomerInfo] }
        return try await send(request("GET", "/customers"), as: List.self).data
    }

    public func createMandate(customerID: String, cap: Money, frequency: MandateFrequency, endDate: String, idempotencyKey: String = UUID().uuidString) async throws -> MandateInfo {
        let body: [String: Any] = ["customer_id": customerID, "max_amount_minor": cap.minor, "frequency": frequency.rawValue, "end_date": endDate]
        let data = try JSONSerialization.data(withJSONObject: body)
        return try await send(request("POST", "/mandates", body: data, idempotencyKey: idempotencyKey), as: MandateInfo.self)
    }

    public func mandates() async throws -> [MandateInfo] {
        struct List: Decodable { let data: [MandateInfo] }
        return try await send(request("GET", "/mandates"), as: List.self).data
    }

    public func mandate(id: String) async throws -> (mandate: MandateInfo, debits: [MandateDebitInfo]) {
        struct Detail: Decodable { let mandate: MandateInfo; let debits: [MandateDebitInfo] }
        let d = try await send(request("GET", "/mandates/\(id)"), as: Detail.self)
        return (d.mandate, d.debits)
    }

    public func refreshMandate(id: String) async throws -> MandateInfo {
        try await send(request("POST", "/mandates/\(id)/refresh"), as: MandateInfo.self)
    }

    public func pauseMandate(id: String) async throws -> MandateInfo { try await send(request("POST", "/mandates/\(id)/pause"), as: MandateInfo.self) }
    public func resumeMandate(id: String) async throws -> MandateInfo { try await send(request("POST", "/mandates/\(id)/resume"), as: MandateInfo.self) }
    public func revokeMandate(id: String) async throws -> MandateInfo { try await send(request("POST", "/mandates/\(id)/revoke"), as: MandateInfo.self) }

    public func scheduleDebit(mandateID: String, amount: Money, due: String, idempotencyKey: String = UUID().uuidString) async throws -> MandateDebitInfo {
        let data = try JSONSerialization.data(withJSONObject: ["amount_minor": amount.minor, "due_date": due])
        return try await send(request("POST", "/mandates/\(mandateID)/debits", body: data, idempotencyKey: idempotencyKey), as: MandateDebitInfo.self)
    }

    /// Run the mandate pass now (a sandbox aid: the in-process simulator only works when the API runs it).
    public func runMandates() async throws -> MandateRunStats {
        try await send(request("POST", "/mandates/run", body: Data("{}".utf8)), as: MandateRunStats.self)
    }
}

public struct CustomerInfo: Codable, Equatable, Sendable, Identifiable { public let id: String; public let name: String }

public struct MandateAuthorization: Codable, Equatable, Hashable, Sendable {
    public let type: String?
    public let uri: String?
}

public struct MandateInfo: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let customerID: String
    public let currency: String
    public let maxAmountMinor: Int64
    public let frequency: String
    public let startDate: String
    public let endDate: String
    public let status: String
    public let authorization: MandateAuthorization?
    public let failureReason: String?
    public let revokedBy: String?
    public let createdAt: String?
    enum CodingKeys: String, CodingKey {
        case id, currency, frequency, status, authorization
        case customerID = "customer_id", maxAmountMinor = "max_amount_minor", startDate = "start_date", endDate = "end_date"
        case failureReason = "failure_reason", revokedBy = "revoked_by", createdAt = "created_at"
    }
    public var cap: Money { Money(minor: maxAmountMinor, currency: currency) }
    public var frequencyTitle: String { MandateFrequency(rawValue: frequency)?.title ?? frequency }
    public var isLive: Bool { ["pending_authorization", "active", "paused"].contains(status) }
}

public struct MandateDebitInfo: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let mandateID: String
    public let amountMinor: Int64
    public let currency: String
    public let dueDate: String
    public let state: String
    public let attemptID: String?
    public let retryNo: Int
    public let failureReason: String?
    enum CodingKeys: String, CodingKey {
        case id, currency, state
        case mandateID = "mandate_id", amountMinor = "amount_minor", dueDate = "due_date", attemptID = "attempt_id", retryNo = "retry_no", failureReason = "failure_reason"
    }
    public var money: Money { Money(minor: amountMinor, currency: currency) }
}

public struct MandateRunStats: Codable, Equatable, Sendable { public let expired: Int; public let notified: Int; public let initiated: Int }

public struct PayerMandateView: Codable, Equatable, Sendable {
    public let mandateID: String
    public let status: String
    public let maxAmountMinor: Int64
    public let debits: [PayerDebit]
    enum CodingKeys: String, CodingKey { case mandateID = "mandate_id", status, maxAmountMinor = "max_amount_minor", debits }
}

public struct BranchInfo: Codable, Equatable, Sendable, Identifiable { public let id: String; public let name: String }

public struct ClosingInfo: Codable, Equatable, Sendable, Identifiable {
    public struct Digital: Codable, Equatable, Sendable { public let method: String; public let payments: Int; public let netMinor: Int64
        enum CodingKeys: String, CodingKey { case method, payments, netMinor = "net_minor" } }
    public let id: String
    public let expectedCashMinor: Int64
    public let countedCashMinor: Int64
    public let differenceMinor: Int64
    public let digital: [Digital]
    public let status: String
    enum CodingKeys: String, CodingKey { case id, digital, status, expectedCashMinor = "expected_cash_minor", countedCashMinor = "counted_cash_minor", differenceMinor = "difference_minor" }
    /// "Short", "Over" or "Matches", in words, so the sign is not the only signal.
    public var verdict: String { differenceMinor == 0 ? "Matches" : (differenceMinor < 0 ? "Short" : "Over") }
}
