import Foundation

/// Your own words on a payment: a note, tags and an optional photo of the bill. Kept on the phone, keyed by the payment's id.
public struct VaultEntry: Codable, Equatable, Sendable {
    public var note = ""
    public var tags: [String] = []
    /// File name inside the app's receipts folder, never a path.
    public var photo: String?
    public init() {}
    public var isEmpty: Bool { note.isEmpty && tags.isEmpty && photo == nil }
}

public struct Vault: Codable, Equatable, Sendable {
    public private(set) var entries: [String: VaultEntry] = [:]
    public init() {}

    public static let maxTags = 8, maxTagLength = 20, maxNoteLength = 280

    /// "Trip, office lunch #tax" becomes ["trip", "office", "lunch", "tax"]: lowercase, split on commas and spaces, no `#`, no repeats, capped.
    public static func parseTags(_ text: String) -> [String] {
        var seen = Set<String>(), out: [String] = []
        for raw in text.lowercased().split(whereSeparator: { $0 == "," || $0 == " " || $0 == "\n" || $0 == ";" }) {
            let t = String(raw.drop(while: { $0 == "#" }).prefix(Self.maxTagLength))
            guard !t.isEmpty, seen.insert(t).inserted else { continue }
            out.append(t); if out.count == Self.maxTags { break }
        }
        return out
    }

    public func entry(_ id: String) -> VaultEntry { entries[id] ?? VaultEntry() }
    public mutating func setNote(_ id: String, _ text: String) { update(id) { $0.note = String(text.trimmingCharacters(in: .whitespacesAndNewlines).prefix(Self.maxNoteLength)) } }
    public mutating func setTags(_ id: String, _ text: String) { update(id) { $0.tags = Self.parseTags(text) } }
    public mutating func setPhoto(_ id: String, _ file: String?) { update(id) { $0.photo = file.flatMap { Self.safeFileName($0) } } }

    /// A plain file name: letters, digits, dash, underscore, one dot. Nothing that could climb out of the receipts folder.
    public static func safeFileName(_ s: String) -> String? {
        guard !s.isEmpty, s.count <= 80, !s.hasPrefix("."), s.filter({ $0 == "." }).count <= 1,
              s.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "-_.".contains($0)) }) else { return nil }
        return s
    }

    private mutating func update(_ id: String, _ f: (inout VaultEntry) -> Void) {
        var e = entry(id); f(&e); entries[id] = e.isEmpty ? nil : e
    }
}

/// A month on one page: what was paid, by category and in full. Plain data, so the PDF drawing stays thin.
public struct MonthReport: Equatable, Sendable {
    public let title: String
    public let totalMinor: Int64
    public let byCategory: [(category: SpendCategory, minor: Int64)]
    public let lines: [HistoryRow]

    public static func == (a: MonthReport, b: MonthReport) -> Bool {
        a.title == b.title && a.totalMinor == b.totalMinor && a.lines == b.lines && a.byCategory.map(\.minor) == b.byCategory.map(\.minor)
    }

    /// Paid payments in the calendar month of `month`, oldest first. Failed payments are not in a report of what was paid.
    public static func make(_ rows: [HistoryRow], month: Date, calendar: Calendar = .current) -> MonthReport {
        let lines = rows.filter { $0.paid && calendar.isDate($0.date, equalTo: month, toGranularity: .month) }.sorted { $0.date < $1.date }
        let entries = lines.map { SpendEntry(amountMinor: $0.amountMinor, category: $0.category, date: $0.date, paid: true) }
        let f = DateFormatter(); f.calendar = calendar; f.timeZone = calendar.timeZone; f.locale = Locale(identifier: "en_IN"); f.dateFormat = "MMMM yyyy"
        return MonthReport(title: f.string(from: month), totalMinor: lines.reduce(0) { $0 + $1.amountMinor },
                           byCategory: MonthSummary.byCategory(entries, now: month, calendar: calendar), lines: lines)
    }
}

/// Split a bill by item. Each item is shared equally by the people it is assigned to; the paise left over go one each to the first people
/// in the list, so the shares always add up to exactly the total.
public enum ItemSplit {
    public struct Item: Equatable, Sendable { public var name: String, amountMinor: Int64, people: Set<Int>
        public init(name: String, amountMinor: Int64, people: Set<Int>) { self.name = name; self.amountMinor = amountMinor; self.people = people } }

    /// One share per person (0..<peopleCount). An item nobody is assigned to is shared by everyone.
    public static func shares(_ items: [Item], peopleCount: Int) -> [Int64] {
        guard peopleCount > 0 else { return [] }
        var out = [Int64](repeating: 0, count: peopleCount)
        for it in items where it.amountMinor > 0 {
            let who = (it.people.isEmpty ? Set(0..<peopleCount) : it.people.filter { (0..<peopleCount).contains($0) }).sorted()
            guard !who.isEmpty else { continue }
            let base = it.amountMinor / Int64(who.count), extra = Int(it.amountMinor % Int64(who.count))
            for (k, p) in who.enumerated() { out[p] += base + (k < extra ? 1 : 0) }
        }
        return out
    }
}
