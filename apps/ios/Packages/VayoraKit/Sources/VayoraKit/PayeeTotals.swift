import Foundation

/// Paid money per payee over a span of days, biggest first. Ties break on the name so the order never shifts between runs.
public struct PayeeTotal: Equatable, Sendable {
    public let payee: String
    public let count: Int
    public let minor: Int64
}

public enum PayeeTotals {
    /// `rows` are history rows; only paid ones count. Payments are grouped by the payee's UPI id (ignoring case and surrounding spaces),
    /// falling back to the name when a row has no id, and shown by the name the person recognises (the first one written), falling back to the id.
    /// A row with neither is counted under "Someone".
    public static func top(_ rows: [HistoryRow], from start: Date, to endExclusive: Date, limit: Int = 5) -> [PayeeTotal] {
        var sums: [String: (shown: String, count: Int, minor: Int64)] = [:]
        for r in rows.sorted(by: { $0.date < $1.date }) where r.paid && r.date >= start && r.date < endExclusive {
            let id = r.payee.trimmingCharacters(in: .whitespaces), name = r.name.trimmingCharacters(in: .whitespaces)
            let key = (id.isEmpty ? name : id).lowercased()
            let shown = name.isEmpty ? (id.isEmpty ? "Someone" : id) : name
            let cur = sums[key] ?? (shown, 0, 0)
            sums[key] = (cur.shown, cur.count + 1, cur.minor + r.amountMinor)
        }
        return sums.values.map { PayeeTotal(payee: $0.shown, count: $0.count, minor: $0.minor) }
            .sorted { $0.minor != $1.minor ? $0.minor > $1.minor : $0.payee < $1.payee }
            .prefix(max(0, limit)).map { $0 }
    }
}
