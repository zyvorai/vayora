import Foundation

public struct PlanInfo: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let name: String
    public let currency: String
    public let amountMinor: Int64
    public let intervalUnit: String
    public let intervalCount: Int
    public let active: Bool
    enum CodingKeys: String, CodingKey { case id, name, currency, active, amountMinor = "amount_minor", intervalUnit = "interval_unit", intervalCount = "interval_count" }
    public init(id: String, name: String, currency: String = "INR", amountMinor: Int64, intervalUnit: String = "month", intervalCount: Int = 1, active: Bool = true) {
        self.id = id; self.name = name; self.currency = currency; self.amountMinor = amountMinor; self.intervalUnit = intervalUnit; self.intervalCount = intervalCount; self.active = active
    }
    /// "₹499.00 every month", "₹1,200.00 every 3 months".
    public var priceLine: String {
        let every = intervalCount == 1 ? "every \(intervalUnit)" : "every \(intervalCount) \(intervalUnit)s"
        return "\(Money(minor: amountMinor).display) \(every)"
    }
}

public struct SubscriptionInfo: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let customerID: String
    public let planID: String
    public let quantity: Int
    public let status: String
    public let nextBillDate: String?
    public let trialEndsOn: String?
    public let unitAmountMinor: Int64?
    enum CodingKeys: String, CodingKey {
        case id, quantity, status
        case customerID = "customer_id", planID = "plan_id", nextBillDate = "next_bill_date", trialEndsOn = "trial_ends_on", unitAmountMinor = "unit_amount_minor"
    }
    public init(id: String, customerID: String = "", planID: String = "", quantity: Int = 1, status: String = "active", nextBillDate: String? = nil, trialEndsOn: String? = nil, unitAmountMinor: Int64? = nil) {
        self.id = id; self.customerID = customerID; self.planID = planID; self.quantity = quantity; self.status = status
        self.nextBillDate = nextBillDate; self.trialEndsOn = trialEndsOn; self.unitAmountMinor = unitAmountMinor
    }
    public var canPause: Bool { status == "active" || status == "trialing" }
    public var canResume: Bool { status == "paused" }
    public var canCancel: Bool { status != "canceled" }
}

/// The test organisation's own date, which billing and reminders follow. Test environments only.
public struct TestClock: Codable, Equatable, Sendable {
    public let date: String
    public let frozen: Bool
    public let realDate: String
    enum CodingKeys: String, CodingKey { case date, frozen, realDate = "real_date" }
}

extension VayoraClient {
    public func plans() async throws -> [PlanInfo] {
        struct List: Decodable { let data: [PlanInfo] }
        return try await send(request("GET", "/plans"), as: List.self).data
    }
    public func subscriptions() async throws -> [SubscriptionInfo] {
        struct List: Decodable { let data: [SubscriptionInfo] }
        return try await send(request("GET", "/subscriptions"), as: List.self).data
    }
    public func pauseSubscription(id: String) async throws { try await subscriptionAction(id, "pause") }
    public func resumeSubscription(id: String) async throws { try await subscriptionAction(id, "resume") }
    /// Ends the subscription. Callers confirm first.
    public func cancelSubscription(id: String) async throws { try await subscriptionAction(id, "cancel") }
    private func subscriptionAction(_ id: String, _ verb: String) async throws {
        struct Done: Decodable {}
        _ = try await send(request("POST", "/subscriptions/\(id)/\(verb)"), as: Done.self)
    }

    public func testClock() async throws -> TestClock { try await send(request("GET", "/test_clock"), as: TestClock.self) }
    /// Moves the clock `days` forward (never back). The server refuses it outside a test environment or for a non-admin key.
    public func advanceTestClock(days: Int) async throws -> TestClock {
        let data = try JSONSerialization.data(withJSONObject: ["advance_days": days])
        return try await send(request("POST", "/test_clock", body: data), as: TestClock.self)
    }
    public func resetTestClock() async throws -> TestClock { try await send(request("DELETE", "/test_clock"), as: TestClock.self) }
}
