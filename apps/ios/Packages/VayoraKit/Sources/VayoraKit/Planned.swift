import Foundation

/// A payment that repeats on its own rhythm, found from history. It only describes; it never pays or schedules anything.
public struct RecurringPayment: Equatable, Sendable {
    public enum Cadence: String, Sendable { case weekly, monthly
        public var title: String { self == .weekly ? "Weekly" : "Monthly" } }
    public let vpa: String, name: String, typicalMinor: Int64, cadence: Cadence, last: Date, next: Date, count: Int
}

public enum RecurringDetector {
    /// Needs three paid payments to the same UPI ID, evenly spaced (weekly: about 7 days; monthly: about 30), with amounts
    /// within 20% of the middle one. AutoPay debits are skipped: they are already managed.
    public static func find(_ history: [HistoryRow], calendar: Calendar = .current) -> [RecurringPayment] {
        var byPayee: [String: [HistoryRow]] = [:]
        for r in history where r.paid && r.kind != "autopay" { byPayee[r.payee.lowercased(), default: []].append(r) }
        var out: [RecurringPayment] = []
        for (vpa, rows) in byPayee where rows.count >= 3 {
            let s = rows.sorted { $0.date < $1.date }
            let amounts = s.map(\.amountMinor).sorted(); let typical = amounts[amounts.count / 2]
            guard typical > 0, s.allSatisfy({ abs(Double($0.amountMinor - typical)) <= 0.2 * Double(typical) }) else { continue }
            let gaps = zip(s, s.dropFirst()).map { calendar.dateComponents([.day], from: calendar.startOfDay(for: $0.date), to: calendar.startOfDay(for: $1.date)).day ?? 0 }
            let med = gaps.sorted()[gaps.count / 2]
            let cadence: RecurringPayment.Cadence
            let tol: Int
            if (5...9).contains(med) { cadence = .weekly; tol = 2 } else if (26...35).contains(med) { cadence = .monthly; tol = 5 } else { continue }
            guard gaps.allSatisfy({ abs($0 - med) <= tol }) else { continue }
            let last = s.last!
            let next = calendar.date(byAdding: .day, value: med, to: last.date)!
            out.append(RecurringPayment(vpa: vpa, name: last.name.isEmpty ? vpa : last.name, typicalMinor: typical, cadence: cadence, last: last.date, next: next, count: s.count))
        }
        return out.sorted { $0.next != $1.next ? $0.next < $1.next : $0.vpa < $1.vpa }
    }
}

/// A one-off payment planned for a date. When it is due the app reminds you; you still slide and enter the PIN.
public struct ScheduledPayment: Codable, Equatable, Identifiable, Sendable {
    public var id = UUID()
    public var vpa: String
    public var name: String
    public var amountMinor: Int64
    public var date: Date
    public var note: String
    public var done = false
    public init(vpa: String, name: String, amountMinor: Int64, date: Date, note: String = "") { self.vpa = vpa; self.name = name; self.amountMinor = amountMinor; self.date = date; self.note = note }
}

public enum ScheduleBook {
    /// Planned and not done, soonest first.
    public static func upcoming(_ all: [ScheduledPayment]) -> [ScheduledPayment] { all.filter { !$0.done }.sorted { $0.date < $1.date } }
    /// Due on or before `now`'s end of day, so a payment planned for today is "due" the whole day.
    public static func due(_ all: [ScheduledPayment], now: Date, calendar: Calendar = .current) -> [ScheduledPayment] {
        let end = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: now))!
        return upcoming(all).filter { $0.date < end }
    }
}

/// Who has been asked to pay their share of a split, and who has said they did. Kept by hand: this phone is not told when another phone pays.
public struct SplitRecord: Codable, Equatable, Identifiable, Sendable {
    public struct Member: Codable, Equatable, Identifiable, Sendable { public var id = UUID(); public var name: String; public var paid = false
        public init(name: String) { self.name = name } }
    public var id = UUID()
    public var date: Date
    public var note: String
    public var totalMinor: Int64
    public var shareMinor: Int64
    public var members: [Member]
    public init(date: Date, note: String, totalMinor: Int64, shareMinor: Int64, people: Int) {
        self.date = date; self.note = note; self.totalMinor = totalMinor; self.shareMinor = shareMinor
        // you are one of the people; the rest owe you
        self.members = (1..<max(2, people)).map { Member(name: "Friend \($0)") }
    }
    public var owedMinor: Int64 { Int64(members.filter { !$0.paid }.count) * shareMinor }
    public var isSettled: Bool { members.allSatisfy(\.paid) }
}
