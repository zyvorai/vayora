import Foundation

/// One line of history in a neutral shape, so filtering and export are testable without the app's types.
public struct HistoryRow: Equatable, Sendable {
    public let date: Date, name: String, payee: String, amountMinor: Int64, paid: Bool, category: SpendCategory, note: String, kind: String
    public init(date: Date, name: String, payee: String, amountMinor: Int64, paid: Bool, category: SpendCategory, note: String, kind: String) {
        self.date = date; self.name = name; self.payee = payee; self.amountMinor = amountMinor; self.paid = paid; self.category = category; self.note = note; self.kind = kind
    }
}

public struct HistoryFilter: Equatable, Sendable {
    public enum Outcome: String, CaseIterable, Sendable { case all, paid, failed }
    public var text = ""
    public var category: SpendCategory?
    public var outcome = Outcome.all
    public var from: Date?
    public var to: Date?
    public init() {}

    public func apply(_ rows: [HistoryRow], calendar: Calendar = .current) -> [HistoryRow] {
        let q = text.trimmingCharacters(in: .whitespaces)
        return rows.filter { r in
            if !q.isEmpty, !(r.name + " " + r.payee + " " + r.note).localizedCaseInsensitiveContains(q) { return false }
            if let c = category, r.category != c { return false }
            switch outcome { case .all: break; case .paid: if !r.paid { return false }; case .failed: if r.paid { return false } }
            if let f = from, r.date < calendar.startOfDay(for: f) { return false }
            if let t = to, r.date >= calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: t))! { return false }
            return true
        }
    }
    public var isActive: Bool { !text.isEmpty || category != nil || outcome != .all || from != nil || to != nil }
}

public enum HistoryCSV {
    /// A cell that starts with `=`, `+`, `-` or `@` would run as a formula in a spreadsheet; a leading `'` keeps it text.
    static func cell(_ s: String) -> String {
        var v = s.replacingOccurrences(of: "\r", with: " ").replacingOccurrences(of: "\n", with: " ")
        if let f = v.first, "=+-@\t".contains(f) { v = "'" + v }
        return (v.contains(",") || v.contains("\"")) ? "\"" + v.replacingOccurrences(of: "\"", with: "\"\"") + "\"" : v
    }

    public static func make(_ rows: [HistoryRow], calendar: Calendar = .current) -> String {
        var out = "date,name,payee,amount_inr,status,category,kind,note\n"
        for r in rows.sorted(by: { $0.date > $1.date }) {
            let c = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: r.date)
            let d = String(format: "%04d-%02d-%02d %02d:%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0, c.hour ?? 0, c.minute ?? 0)
            out += [d, cell(r.name), cell(r.payee), Money(minor: r.amountMinor).decimalString, r.paid ? "paid" : "failed", r.category.rawValue, cell(r.kind), cell(r.note)].joined(separator: ",") + "\n"
        }
        return out
    }
}
