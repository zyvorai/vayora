import Foundation

/// Why money was given back. The raw value is what the server stores in the refund's `reason`.
public enum RefundReason: String, Codable, CaseIterable, Sendable {
    case customerRequest = "requested_by_customer"
    case wrongItem = "wrong_item"
    case quality = "quality_issue"
    case duplicate = "duplicate"
    case other = "requested_by_merchant"

    public var title: String {
        switch self {
        case .customerRequest: "Customer asked"
        case .wrongItem: "Wrong item"
        case .quality: "Quality problem"
        case .duplicate: "Charged twice"
        case .other: "Other"
        }
    }
}

/// A refund this phone made, kept with the reason and an optional note.
public struct RefundEntry: Codable, Equatable, Sendable {
    public let intentID: String
    public let date: Date
    public let amountMinor: Int64
    public let reason: RefundReason
    public let note: String
    public init(intentID: String, date: Date, amountMinor: Int64, reason: RefundReason, note: String) {
        self.intentID = intentID; self.date = date; self.amountMinor = amountMinor; self.reason = reason
        self.note = String(note.trimmingCharacters(in: .whitespacesAndNewlines).prefix(200))
    }
}

public enum RefundPattern {
    /// Refunds since `since`, most common reason first (ties in the order of `RefundReason.allCases`).
    public static func counts(_ log: [RefundEntry], since: Date) -> [(reason: RefundReason, count: Int)] {
        let recent = log.filter { $0.date >= since }
        return RefundReason.allCases.compactMap { r in
            let n = recent.filter { $0.reason == r }.count
            return n > 0 ? (r, n) : nil
        }.sorted { $0.count > $1.count }
    }

    /// "3 refunds in 7 days (2 Customer asked, 1 Wrong item) out of 40 paid." or nil when there were none.
    public static func line(_ log: [RefundEntry], since: Date, days: Int, paidCount: Int) -> String? {
        let c = counts(log, since: since)
        let total = c.reduce(0) { $0 + $1.count }
        guard total > 0 else { return nil }
        let parts = c.map { "\($0.count) \($0.reason.title)" }.joined(separator: ", ")
        let out = paidCount > 0 ? " out of \(paidCount) paid" : ""
        return "\(total) refund\(total == 1 ? "" : "s") in \(days) days (\(parts))\(out)."
    }
}
