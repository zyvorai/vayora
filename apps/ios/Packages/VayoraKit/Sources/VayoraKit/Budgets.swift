import Foundation

/// A monthly limit per category, in paise. A category with no limit is simply not watched.
public struct Budgets: Codable, Equatable, Sendable {
    public var limits: [String: Int64] = [:]
    public init(limits: [SpendCategory: Int64] = [:]) { self.limits = Dictionary(uniqueKeysWithValues: limits.filter { $0.value > 0 }.map { ($0.key.rawValue, $0.value) }) }

    public func limit(_ c: SpendCategory) -> Int64? { limits[c.rawValue] }
    public mutating func set(_ c: SpendCategory, minor: Int64?) {
        if let m = minor, m > 0 { limits[c.rawValue] = m } else { limits[c.rawValue] = nil }
    }
}

public struct BudgetStatus: Equatable, Sendable, Identifiable {
    public enum Level: String, Sendable { case ok, near, over }
    public let category: SpendCategory, spentMinor: Int64, limitMinor: Int64
    public var id: String { category.rawValue }
    /// Spent over limit, so 0.5 is half. May exceed 1.
    public var fraction: Double { Double(spentMinor) / Double(limitMinor) }
    /// "Near" from 80% so there is time to slow down; "over" strictly above the limit.
    public var level: Level { spentMinor > limitMinor ? .over : (Double(spentMinor) >= 0.8 * Double(limitMinor) ? .near : .ok) }
    public var leftMinor: Int64 { max(0, limitMinor - spentMinor) }
}

public enum BudgetCheck {
    /// One status per watched category, for the month of `now`: worst first, then by name.
    public static func statuses(_ entries: [SpendEntry], budgets: Budgets, now: Date, calendar: Calendar = .current) -> [BudgetStatus] {
        let spent = Dictionary(uniqueKeysWithValues: MonthSummary.byCategory(entries, now: now, calendar: calendar).map { ($0.category, $0.minor) })
        return SpendCategory.allCases.compactMap { c in
            guard let l = budgets.limit(c) else { return nil }
            return BudgetStatus(category: c, spentMinor: spent[c] ?? 0, limitMinor: l)
        }.sorted { $0.fraction != $1.fraction ? $0.fraction > $1.fraction : $0.category.rawValue < $1.category.rawValue }
    }

    /// Would paying `amount` in `category` push it over (or newly near) its limit? Used for a gentle warning before the slide; it never blocks a payment.
    public static func warning(adding amount: Int64, to category: SpendCategory, entries: [SpendEntry], budgets: Budgets, now: Date, calendar: Calendar = .current) -> BudgetStatus? {
        guard let s = statuses(entries, budgets: budgets, now: now, calendar: calendar).first(where: { $0.category == category }) else { return nil }
        let after = BudgetStatus(category: category, spentMinor: s.spentMinor + amount, limitMinor: s.limitMinor)
        return after.level == .ok ? nil : after
    }
}
