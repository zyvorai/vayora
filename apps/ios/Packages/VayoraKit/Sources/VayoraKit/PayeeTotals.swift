import Foundation

/// Paid money per payee over a span of days, biggest first. Ties break on the name so the order never shifts between runs.
public struct PayeeTotal: Equatable, Sendable {
    public let payee: String
    public let count: Int
    public let minor: Int64
}

public enum PayeeTotals {
    /// `rows` are history rows; only paid ones count. A payee is matched ignoring case and surrounding spaces,
    /// and is shown the way it was first written. Rows without a payee name are counted under `name`, then "Someone".
    public static func top(_ rows: [HistoryRow], from start: Date, to endExclusive: Date, limit: Int = 5) -> [PayeeTotal] {
        var sums: [String: (shown: String, count: Int, minor: Int64)] = [:]
        for r in rows.sorted(by: { $0.date < $1.date }) where r.paid && r.date >= start && r.date < endExclusive {
            let raw = [r.payee, r.name].map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? "Someone"
            let key = raw.lowercased()
            let cur = sums[key] ?? (raw, 0, 0)
            sums[key] = (cur.shown, cur.count + 1, cur.minor + r.amountMinor)
        }
        return sums.values.map { PayeeTotal(payee: $0.shown, count: $0.count, minor: $0.minor) }
            .sorted { $0.minor != $1.minor ? $0.minor > $1.minor : $0.payee < $1.payee }
            .prefix(max(0, limit)).map { $0 }
    }
}
