import Foundation

public enum SpendCategory: String, Codable, CaseIterable, Sendable {
    case food, shopping, bills, travel, transfers, other

    public var title: String {
        switch self { case .food: "Food"; case .shopping: "Shopping"; case .bills: "Bills"; case .travel: "Travel"; case .transfers: "Transfers"; case .other: "Other" }
    }

    /// A best guess from the payee's name and the note; the person can change it. Never decides anything about money.
    public static func guess(name: String, note: String, transfer: Bool) -> SpendCategory {
        if transfer { return .transfers }
        let t = (name + " " + note).lowercased()
        let table: [(SpendCategory, [String])] = [
            (.food, ["cafe", "chai", "tea", "coffee", "restaurant", "food", "pizza", "bakery", "dhaba", "swiggy", "zomato"]),
            (.bills, ["electric", "water", "gas", "broadband", "recharge", "rent", "bill", "insurance", "dth"]),
            (.travel, ["cab", "uber", "ola", "metro", "bus", "train", "fuel", "petrol", "flight", "toll"]),
            (.shopping, ["store", "mart", "shop", "mall", "market", "stores", "fashion"]),
        ]
        for (c, words) in table where words.contains(where: t.contains) { return c }
        return .other
    }
}

public struct SpendEntry: Equatable, Sendable {
    public let amountMinor: Int64, category: SpendCategory, date: Date, paid: Bool
    public init(amountMinor: Int64, category: SpendCategory, date: Date, paid: Bool) { self.amountMinor = amountMinor; self.category = category; self.date = date; self.paid = paid }
}

public enum MonthSummary {
    /// Paid spending in the calendar month of `now`, per category, biggest first. Failed payments never count.
    public static func byCategory(_ entries: [SpendEntry], now: Date, calendar: Calendar = .current) -> [(category: SpendCategory, minor: Int64)] {
        var sums: [SpendCategory: Int64] = [:]
        for e in entries where e.paid && calendar.isDate(e.date, equalTo: now, toGranularity: .month) { sums[e.category, default: 0] += e.amountMinor }
        return sums.map { (category: $0.key, minor: $0.value) }.sorted { $0.minor != $1.minor ? $0.minor > $1.minor : $0.category.rawValue < $1.category.rawValue }
    }
    public static func total(_ entries: [SpendEntry], now: Date, calendar: Calendar = .current) -> Int64 { byCategory(entries, now: now, calendar: calendar).reduce(0) { $0 + $1.minor } }
}

/// A bill that comes round every month. Day 1 to 28 so it exists in every month.
public struct BillReminder: Codable, Equatable, Identifiable, Sendable {
    public var id = UUID()
    public var name: String
    public var amountMinor: Int64
    public var dayOfMonth: Int
    public var enabled = true
    public init(name: String, amountMinor: Int64, dayOfMonth: Int) { self.name = name; self.amountMinor = amountMinor; self.dayOfMonth = min(28, max(1, dayOfMonth)) }

    /// The next time this falls due, at 9 am, strictly after `date`.
    public func nextDue(after date: Date, calendar: Calendar = .current) -> Date {
        var c = calendar.dateComponents([.year, .month], from: date); c.day = dayOfMonth; c.hour = 9; c.minute = 0
        let thisMonth = calendar.date(from: c)!
        if thisMonth > date { return thisMonth }
        return calendar.date(byAdding: .month, value: 1, to: thisMonth)!
    }
}
