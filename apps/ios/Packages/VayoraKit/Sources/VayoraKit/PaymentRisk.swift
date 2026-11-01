import Foundation

public enum RiskFlag: Equatable, Sendable {
    /// Never paid before and not saved.
    case firstTime
    /// One edit away from a saved payee, so a slip or a lookalike.
    case lookalike(of: String)
    /// Much more than this payee has ever been paid before.
    case unusuallyLarge(typicalMinor: Int64)
}

/// Checks run before the slide. They only ever warn: the person decides.
public enum PaymentRisk {
    public static func check(payee rawVPA: String, amountMinor: Int64, history: [HistoryRow], saved: [Payee]) -> [RiskFlag] {
        let vpa = rawVPA.lowercased()
        let isSaved = saved.contains { $0.vpa == vpa }
        let past = history.filter { $0.payee.lowercased() == vpa && $0.paid }.map(\.amountMinor).sorted()
        var flags: [RiskFlag] = []
        if past.isEmpty && !isSaved { flags.append(.firstTime) }
        if !isSaved, let twin = saved.first(where: { $0.vpa != vpa && vpa.count >= 6 && editDistance($0.vpa, vpa) == 1 }) { flags.append(.lookalike(of: twin.vpa)) }
        if past.count >= 3 {
            let median = past[past.count / 2]
            if median > 0, amountMinor > median * 5 { flags.append(.unusuallyLarge(typicalMinor: median)) }
        }
        return flags
    }

    /// A short pause before a big payment to someone new, so a scam or a typo has time to be noticed. 0 means none.
    public static func coolingSeconds(_ flags: [RiskFlag], amountMinor: Int64) -> Int {
        flags.contains(.firstTime) && amountMinor >= 5_000_00 ? 10 : 0
    }

    /// Levenshtein distance, the number of single-character edits between two strings.
    public static func editDistance(_ a: String, _ b: String) -> Int {
        let x = Array(a), y = Array(b)
        if x.isEmpty { return y.count }; if y.isEmpty { return x.count }
        var prev = Array(0...y.count)
        for i in 1...x.count {
            var cur = [i] + [Int](repeating: 0, count: y.count)
            for j in 1...y.count { cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x[i - 1] == y[j - 1] ? 0 : 1)) }
            prev = cur
        }
        return prev[y.count]
    }
}

/// The payees you usually pay at about this time, for a one-tap row on Home.
public enum PayAgain {
    public struct Suggestion: Equatable, Sendable { public let vpa: String, name: String, count: Int }

    /// Needs two paid payments to a payee. A payment on the same weekday, or within two hours of now, counts extra.
    public static func suggest(_ history: [HistoryRow], now: Date, limit: Int = 3, calendar: Calendar = .current) -> [Suggestion] {
        var byPayee: [String: (name: String, count: Int, score: Double, last: Date)] = [:]
        let wd = calendar.component(.weekday, from: now), hr = calendar.component(.hour, from: now)
        for r in history where r.paid && r.kind != "autopay" {
            let k = r.payee.lowercased()
            var e = byPayee[k] ?? (r.name, 0, 0, .distantPast)
            e.count += 1
            e.score += 1
            if calendar.component(.weekday, from: r.date) == wd { e.score += 1 }
            let dh = abs(calendar.component(.hour, from: r.date) - hr); if min(dh, 24 - dh) <= 2 { e.score += 1 }
            if r.date > e.last { e.last = r.date; if !r.name.isEmpty { e.name = r.name } }
            byPayee[k] = e
        }
        return byPayee.filter { $0.value.count >= 2 }
            .sorted { $0.value.score != $1.value.score ? $0.value.score > $1.value.score : ($0.value.last != $1.value.last ? $0.value.last > $1.value.last : $0.key < $1.key) }
            .prefix(limit).map { Suggestion(vpa: $0.key, name: $0.value.name.isEmpty ? $0.key : $0.value.name, count: $0.value.count) }
    }
}
