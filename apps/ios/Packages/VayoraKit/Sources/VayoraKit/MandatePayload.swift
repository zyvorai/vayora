import Foundation

public enum MandateFrequency: String, CaseIterable, Codable, Sendable {
    case daily, weekly, fortnightly, monthly, bimonthly, quarterly
    case halfYearly = "half_yearly", yearly
    case asPresented = "as_presented"

    public var title: String {
        switch self {
        case .daily: "Every day"
        case .weekly: "Every week"
        case .fortnightly: "Every 2 weeks"
        case .monthly: "Every month"
        case .bimonthly: "Every 2 months"
        case .quarterly: "Every 3 months"
        case .halfYearly: "Every 6 months"
        case .yearly: "Every year"
        case .asPresented: "Whenever the merchant asks"
        }
    }
}

/// A parsed `upi://mandate?...` link: a payer being asked to let a merchant debit them on a schedule.
/// Anything scanned from a QR code is hostile until proven otherwise, and the mandate id ends up in a URL path,
/// so it is held to a strict character set.
public struct MandatePayload: Equatable, Sendable {
    public let payee: String        // pa
    public let name: String         // pn
    public let cap: Money           // am: the most one debit may take
    public let mandateID: String    // tn
    public let frequency: MandateFrequency
    public let start: Date?
    public let end: Date?

    public enum Failure: Error, Equatable {
        case notMandate, missingPayee, badPayee, badAmount, unsupportedCurrency, badMandateID, badFrequency, badDate, duplicateParameter
    }

    public static func parse(_ text: String) throws -> MandatePayload {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.lowercased().hasPrefix("upi://mandate?"), let comps = URLComponents(string: t) else { throw Failure.notMandate }
        var q: [String: String] = [:]
        for item in comps.queryItems ?? [] {
            if q[item.name] != nil { throw Failure.duplicateParameter }
            q[item.name] = item.value ?? ""
        }
        guard let pa = q["pa"], !pa.isEmpty else { throw Failure.missingPayee }
        guard UPIPayload.isValidVPA(pa) else { throw Failure.badPayee }
        if let cu = q["cu"], cu.uppercased() != "INR" { throw Failure.unsupportedCurrency }
        guard let am = q["am"], let cap = Money(decimal: am) else { throw Failure.badAmount }
        guard let id = q["tn"], Self.isSafeID(id) else { throw Failure.badMandateID }
        guard let f = q["recur"].flatMap(MandateFrequency.init(rawValue:)) else { throw Failure.badFrequency }
        let start = try q["start"].map { try Self.date($0) }
        let end = try q["end"].map { try Self.date($0) }
        if let s = start, let e = end, e <= s { throw Failure.badDate }
        return MandatePayload(payee: pa, name: q["pn"] ?? "", cap: cap, mandateID: id, frequency: f, start: start, end: end)
    }

    /// Letters, digits and underscores only, so an id can never reshape a URL path.
    static func isSafeID(_ s: String) -> Bool {
        (4...64).contains(s.count) && s.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") }
    }

    /// `yyyy-MM-dd` as a calendar day, held at midday in `calendar` (the viewer's own by default) so it displays as
    /// the day it names, in whatever time zone the phone is in.
    public static func date(_ s: String, calendar: Calendar = .current) throws -> Date {
        let p = s.split(separator: "-", omittingEmptySubsequences: false)
        guard p.count == 3, p[0].count == 4, p[1].count == 2, p[2].count == 2,
              let y = Int(p[0]), let m = Int(p[1]), let d = Int(p[2]) else { throw Failure.badDate }
        guard let date = calendar.date(from: DateComponents(year: y, month: m, day: d, hour: 12)),
              calendar.dateComponents([.year, .month, .day], from: date) == DateComponents(year: y, month: m, day: d) else { throw Failure.badDate }
        return date
    }
}
