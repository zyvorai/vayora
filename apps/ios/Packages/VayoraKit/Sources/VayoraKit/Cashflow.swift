import Foundation

/// What is owed, by due week, plus what is overdue, promised and waiting to be paid out. A schedule, not a prediction.
public struct Cashflow: Codable, Equatable, Sendable {
    public struct Week: Codable, Equatable, Sendable, Identifiable {
        public let starts: String
        public let dueMinor: Int64
        public var id: String { starts }
        enum CodingKeys: String, CodingKey { case starts, dueMinor = "due_minor" }
        public init(starts: String, dueMinor: Int64) { self.starts = starts; self.dueMinor = dueMinor }
    }
    public let asOf: String?
    public let weeks: [Week]
    public let overdueMinor: Int64
    public let dueLaterMinor: Int64
    public let noDueDateMinor: Int64
    public let promisedMinor: Int64
    public let payoutsWaitingMinor: Int64
    public let dailyReceiptsRunRateMinor: Int64
    enum CodingKeys: String, CodingKey {
        case weeks, asOf = "as_of", overdueMinor = "overdue_minor", dueLaterMinor = "due_later_minor", noDueDateMinor = "no_due_date_minor"
        case promisedMinor = "promised_minor", payoutsWaitingMinor = "payouts_waiting_minor", dailyReceiptsRunRateMinor = "daily_receipts_run_rate_minor"
    }
    public init(asOf: String? = nil, weeks: [Week] = [], overdueMinor: Int64 = 0, dueLaterMinor: Int64 = 0, noDueDateMinor: Int64 = 0, promisedMinor: Int64 = 0, payoutsWaitingMinor: Int64 = 0, dailyReceiptsRunRateMinor: Int64 = 0) {
        self.asOf = asOf; self.weeks = weeks; self.overdueMinor = overdueMinor; self.dueLaterMinor = dueLaterMinor; self.noDueDateMinor = noDueDateMinor
        self.promisedMinor = promisedMinor; self.payoutsWaitingMinor = payoutsWaitingMinor; self.dailyReceiptsRunRateMinor = dailyReceiptsRunRateMinor
    }
    /// Everything owed, whenever it is due.
    public var totalOwedMinor: Int64 { overdueMinor + weeks.reduce(0) { $0 + $1.dueMinor } + dueLaterMinor + noDueDateMinor }
}

/// A bank settlement from a payment provider.
public struct SettlementInfo: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let provider: String
    public let utr: String?
    public let grossMinor: Int64
    public let refundsMinor: Int64
    public let feeMinor: Int64
    public let adjustmentMinor: Int64
    public let netMinor: Int64
    public let settledOn: String?
    public let status: String
    enum CodingKeys: String, CodingKey {
        case id, provider, utr, status
        case grossMinor = "gross_minor", refundsMinor = "refunds_minor", feeMinor = "fee_minor", adjustmentMinor = "adjustment_minor", netMinor = "net_minor", settledOn = "settled_on"
    }
    public init(id: String, provider: String, utr: String? = nil, grossMinor: Int64, refundsMinor: Int64 = 0, feeMinor: Int64 = 0, adjustmentMinor: Int64 = 0, netMinor: Int64, settledOn: String? = nil, status: String = "matched") {
        self.id = id; self.provider = provider; self.utr = utr; self.grossMinor = grossMinor; self.refundsMinor = refundsMinor; self.feeMinor = feeMinor
        self.adjustmentMinor = adjustmentMinor; self.netMinor = netMinor; self.settledOn = settledOn; self.status = status
    }
    /// What the provider kept or took back between gross and net.
    public var deductionsMinor: Int64 { grossMinor - netMinor }
}

extension VayoraClient {
    public func cashflow(weeks: Int = 4) async throws -> Cashflow { try await send(request("GET", "/insights/cashflow?weeks=\(max(1, min(12, weeks)))"), as: Cashflow.self) }

    public func settlements() async throws -> [SettlementInfo] {
        struct List: Decodable { let data: [SettlementInfo] }
        return try await send(request("GET", "/settlements"), as: List.self).data
    }
}
