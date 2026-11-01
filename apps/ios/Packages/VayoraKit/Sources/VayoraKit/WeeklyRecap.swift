import Foundation

/// A week of the phone's own payment history, in words. Everything comes from the history on the phone; nothing is estimated.
public struct WeeklyRecap: Equatable, Sendable {
    public let weekStart: Date
    public let weekEndExclusive: Date
    public let paidCount: Int
    public let totalMinor: Int64
    /// The week before, for comparison. Nil when that week had no paid payments.
    public let previousTotalMinor: Int64?
    public let topCategory: (category: SpendCategory, minor: Int64)?
    public let biggest: (name: String, minor: Int64)?
    /// Days from the start of the week up to and including today (or the whole week, for a past week) with no paid payment.
    public let quietDays: Int
    public let daysCounted: Int

    public static func == (a: WeeklyRecap, b: WeeklyRecap) -> Bool {
        a.weekStart == b.weekStart && a.paidCount == b.paidCount && a.totalMinor == b.totalMinor && a.previousTotalMinor == b.previousTotalMinor
            && a.topCategory?.category == b.topCategory?.category && a.topCategory?.minor == b.topCategory?.minor
            && a.biggest?.name == b.biggest?.name && a.biggest?.minor == b.biggest?.minor && a.quietDays == b.quietDays && a.daysCounted == b.daysCounted
    }

    public var isEmpty: Bool { paidCount == 0 }

    /// Change against the week before as a whole percent, or nil when there is nothing to compare with.
    public var changePercent: Int? {
        guard let p = previousTotalMinor, p > 0 else { return nil }
        let diff = Double(totalMinor - p) / Double(p) * 100
        return Int(diff.rounded())
    }

    /// `calendar.firstWeekday` decides where the week starts, so it follows the phone's region. `now` bounds "days counted".
    public static func make(_ rows: [HistoryRow], weekContaining day: Date, now: Date = Date(), calendar: Calendar = .current) -> WeeklyRecap {
        let start = calendar.dateInterval(of: .weekOfYear, for: day)?.start ?? calendar.startOfDay(for: day)
        let end = calendar.date(byAdding: .day, value: 7, to: start) ?? start
        let prevStart = calendar.date(byAdding: .day, value: -7, to: start) ?? start
        let paid = rows.filter { $0.paid }
        let inWeek = paid.filter { $0.date >= start && $0.date < end }
        let prev = paid.filter { $0.date >= prevStart && $0.date < start }.reduce(Int64(0)) { $0 + $1.amountMinor }

        var byCat: [SpendCategory: Int64] = [:]
        for r in inWeek { byCat[r.category, default: 0] += r.amountMinor }
        // ties go to the category that comes first in the list, so the answer does not change between runs
        let top = SpendCategory.allCases.compactMap { c in byCat[c].map { (c, $0) } }.max { $0.1 < $1.1 }

        let big = inWeek.max { $0.amountMinor < $1.amountMinor }
        let lastDay = min(end, calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: now)) ?? end)
        var counted = 0, quiet = 0
        var d = start
        while d < lastDay {
            counted += 1
            let next = calendar.date(byAdding: .day, value: 1, to: d) ?? lastDay
            if !inWeek.contains(where: { $0.date >= d && $0.date < next }) { quiet += 1 }
            d = next
        }
        return WeeklyRecap(weekStart: start, weekEndExclusive: end, paidCount: inWeek.count, totalMinor: inWeek.reduce(0) { $0 + $1.amountMinor },
                           previousTotalMinor: prev > 0 ? prev : nil,
                           topCategory: top.map { (category: $0.0, minor: $0.1) },
                           biggest: big.map { (name: $0.name.isEmpty ? $0.payee : $0.name, minor: $0.amountMinor) },
                           quietDays: quiet, daysCounted: counted)
    }

    /// Plain sentences for the screen and for sharing.
    public var lines: [String] {
        guard !isEmpty else { return ["No payments this week yet."] }
        var out = ["You paid \(Money(minor: totalMinor).display) in \(paidCount) payment\(paidCount == 1 ? "" : "s")."]
        if let c = changePercent {
            out.append(c == 0 ? "That is the same as the week before." : "That is \(abs(c))% \(c > 0 ? "more" : "less") than the week before.")
        }
        if let t = topCategory { out.append("Most went to \(t.category.title): \(Money(minor: t.minor).display).") }
        if let b = biggest { out.append("Biggest payment: \(Money(minor: b.minor).display) to \(b.name).") }
        if daysCounted > 0 { out.append(quietDays == 0 ? "You paid on every day so far." : "\(quietDays) of \(daysCounted) day\(daysCounted == 1 ? "" : "s") with no payments.") }
        out.append("Test money from this phone's history.")
        return out
    }
}
