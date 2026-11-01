import Foundation

public enum Tip {
    public static let percents = [5, 10, 15]
    /// Tip for a base amount at a whole percent, rounded half up to the paisa. Never negative, zero for no base.
    public static func amount(base: Int64, percent: Int) -> Int64 {
        guard base > 0, percent > 0 else { return 0 }
        return (base * Int64(percent) + 50) / 100
    }
}

public enum HourlyTotals {
    /// Paid money per hour (0...23) for the calendar day of `day`. `payments` is (when, minor); only that day counts.
    public static func make(_ payments: [(Date, Int64)], day: Date, calendar: Calendar = .current) -> [Int64] {
        var out = [Int64](repeating: 0, count: 24)
        for (d, m) in payments where calendar.isDate(d, inSameDayAs: day) { out[calendar.component(.hour, from: d)] += m }
        return out
    }
    /// The hour range worth drawing: from the first to the last hour with money, at least 6 wide, so one payment is not a full-width bar.
    public static func window(_ totals: [Int64]) -> ClosedRange<Int> {
        guard let first = totals.firstIndex(where: { $0 > 0 }), let last = totals.lastIndex(where: { $0 > 0 }) else { return 9...17 }
        var lo = first, hi = last
        while hi - lo + 1 < 6 { if lo > 0 { lo -= 1 }; if hi - lo + 1 < 6, hi < 23 { hi += 1 } }
        return lo...hi
    }
    public static func label(_ hour: Int) -> String { hour == 0 ? "12a" : hour < 12 ? "\(hour)a" : hour == 12 ? "12p" : "\(hour - 12)p" }
}

public enum ReceiptText {
    /// Plain text a merchant can share. Only what the server knows: amount, status, id, time. No card or payer details are invented.
    public static func make(merchant: String, amount: Money, status: String, id: String, when: Date?, refundedMinor: Int64 = 0, calendar: Calendar = .current) -> String {
        var lines = [merchant.isEmpty ? "Payment receipt" : "Receipt from \(merchant)", "Amount: \(amount.display)", "Status: \(status == "succeeded" ? "Paid" : status)"]
        if refundedMinor > 0 { lines.append("Refunded: \(Money(minor: refundedMinor, currency: amount.currency).display)") }
        if let w = when { lines.append("Date: " + w.formatted(.dateTime.day().month(.abbreviated).year().hour().minute())) }
        lines.append("Reference: \(id)")
        lines.append("Test payment: no real money moved.")
        return lines.joined(separator: "\n")
    }
}
