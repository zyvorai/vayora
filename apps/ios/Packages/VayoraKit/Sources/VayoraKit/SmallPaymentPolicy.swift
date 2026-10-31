import Foundation

/// The UPI Lite idea, in test mode: payments up to a small limit need only the slide, not the PIN.
/// A payment needs the PIN when the feature is off, the amount is over the per-payment limit, or it would take
/// today's no-PIN total over the daily limit.
public struct SmallPaymentPolicy: Equatable, Sendable {
    public let perPaymentMinor: Int64
    public let dailyMinor: Int64
    public init(perPaymentMinor: Int64 = 20_000, dailyMinor: Int64 = 100_000) { self.perPaymentMinor = perPaymentMinor; self.dailyMinor = dailyMinor }

    public func needsPIN(amount: Money, noPINSpentTodayMinor: Int64, enabled: Bool) -> Bool {
        guard enabled, amount.currency == "INR", amount.minor > 0 else { return true }
        if amount.minor > perPaymentMinor { return true }
        return noPINSpentTodayMinor + amount.minor > dailyMinor
    }
}
