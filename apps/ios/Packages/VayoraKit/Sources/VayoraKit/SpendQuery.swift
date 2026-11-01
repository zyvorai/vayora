import Foundation

/// "food last week", "more than 500 this month", "chai point": plain words turned into a filter and a total.
/// Deterministic: no model, no network. Words it does not understand become a name search.
public struct SpendQuery: Equatable, Sendable {
    public var filter = HistoryFilter()
    public var minMinor: Int64?
    public var maxMinor: Int64?
    public var periodLabel: String?
    /// True when at least a period, a category, an amount rule or "failed" was understood, so the answer card is worth showing.
    public var isStructured: Bool { periodLabel != nil || filter.category != nil || minMinor != nil || maxMinor != nil || filter.outcome == .failed }

    static let stop: Set<String> = ["how", "much", "did", "i", "spend", "spent", "on", "in", "total", "for", "the", "my", "paid", "pay", "to", "at", "of", "a", "was", "were", "what", "show", "me", "all", "payments", "payment", "than", "last", "this", "past", "days", "day", "week", "month", "year", "today", "yesterday", "more", "less", "over", "under", "above", "below", "between", "failed", "and"]
    static let categories: [(SpendCategory, [String])] = [
        (.food, ["food", "eating", "restaurant", "restaurants", "groceries"]),
        (.shopping, ["shopping", "shop", "shops"]),
        (.bills, ["bills", "bill", "rent", "electricity", "recharge"]),
        (.travel, ["travel", "cab", "cabs", "fuel", "petrol", "transport"]),
        (.transfers, ["transfers", "transfer", "friends", "sent"]),
        (.other, ["other"]),
    ]

    public static func parse(_ text: String, now: Date = Date(), calendar: Calendar = .current) -> SpendQuery {
        var q = SpendQuery()
        let lower = text.lowercased()
        let words = lower.split { !$0.isLetter && !$0.isNumber && $0 != "." }.map(String.init)
        var used = Set<Int>()

        func range(_ interval: DateInterval, _ label: String) {
            q.filter.from = interval.start; q.filter.to = calendar.date(byAdding: .second, value: -1, to: interval.end); q.periodLabel = label
        }
        func has(_ phrase: String) -> Bool { lower.contains(phrase) }
        let day = calendar.startOfDay(for: now)
        if has("yesterday") { let y = calendar.date(byAdding: .day, value: -1, to: day)!; range(DateInterval(start: y, end: day), "yesterday") }
        else if has("today") { range(DateInterval(start: day, end: calendar.date(byAdding: .day, value: 1, to: day)!), "today") }
        else if has("last week"), let w = calendar.dateInterval(of: .weekOfYear, for: calendar.date(byAdding: .day, value: -7, to: now)!) { range(w, "last week") }
        else if has("this week"), let w = calendar.dateInterval(of: .weekOfYear, for: now) { range(w, "this week") }
        else if has("last month"), let m = calendar.dateInterval(of: .month, for: calendar.date(byAdding: .month, value: -1, to: now)!) { range(m, "last month") }
        else if has("this month"), let m = calendar.dateInterval(of: .month, for: now) { range(m, "this month") }
        else if has("this year"), let y = calendar.dateInterval(of: .year, for: now) { range(y, "this year") }
        else if let i = words.firstIndex(where: { $0 == "days" }), i >= 1, let n = Int(words[i - 1]), (1...365).contains(n) {
            range(DateInterval(start: calendar.date(byAdding: .day, value: -(n - 1), to: day)!, end: calendar.date(byAdding: .day, value: 1, to: day)!), "last \(n) days")
        }

        // an amount rule: "more than 500", "over 500", "under 200"
        for (i, w) in words.enumerated() where i + 1 < words.count {
            let next = (words[i + 1] == "than" && i + 2 < words.count) ? i + 2 : i + 1
            guard let n = Money(decimal: words[next]), ["more", "over", "above", "less", "under", "below"].contains(w) else { continue }
            if ["more", "over", "above"].contains(w) { q.minMinor = n.minor } else { q.maxMinor = n.minor }
            used.insert(next)
        }
        if has("failed") { q.filter.outcome = .failed }

        for (i, w) in words.enumerated() {
            if let c = categories.first(where: { $0.1.contains(w) }) { if q.filter.category == nil { q.filter.category = c.0 }; used.insert(i) }
        }
        let rest = words.enumerated().filter { !used.contains($0.offset) && !stop.contains($0.element) && Int($0.element) == nil && Double($0.element) == nil }.map(\.element)
        q.filter.text = rest.joined(separator: " ")
        return q
    }

    public func apply(_ rows: [HistoryRow], calendar: Calendar = .current) -> [HistoryRow] {
        filter.apply(rows, calendar: calendar).filter { r in
            if let m = minMinor, r.amountMinor <= m { return false }
            if let m = maxMinor, r.amountMinor >= m { return false }
            return true
        }
    }

    /// Money out (paid only) and how many payments, over the matching rows.
    public static func total(_ rows: [HistoryRow]) -> (minor: Int64, count: Int) {
        let paid = rows.filter(\.paid); return (paid.reduce(0) { $0 + $1.amountMinor }, paid.count)
    }
}
