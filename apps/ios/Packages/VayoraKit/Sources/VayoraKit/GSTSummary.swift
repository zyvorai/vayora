import Foundation

/// A calendar month in Indian time, as the GST report asks for it ("2026-10").
public struct GSTPeriod: Equatable, Sendable {
    public let year: Int, month: Int
    public init(year: Int, month: Int) { self.year = year; self.month = month }

    public var id: String { String(format: "%04d-%02d", year, month) }

    /// The month `date` falls in, in Indian time.
    public static func containing(_ date: Date) -> GSTPeriod {
        var cal = Calendar(identifier: .gregorian); cal.timeZone = TimeZone(identifier: "Asia/Kolkata")!
        let c = cal.dateComponents([.year, .month], from: date)
        return GSTPeriod(year: c.year ?? 1970, month: c.month ?? 1)
    }
    public func shifted(by months: Int) -> GSTPeriod {
        let index = year * 12 + (month - 1) + months
        return GSTPeriod(year: index / 12, month: index % 12 + 1)
    }
    /// "October 2026".
    public var title: String {
        let f = DateFormatter(); f.locale = Locale(identifier: "en_IN"); f.dateFormat = "LLLL yyyy"
        var comps = DateComponents(); comps.year = year; comps.month = month; comps.day = 1
        var cal = Calendar(identifier: .gregorian); cal.timeZone = TimeZone(identifier: "Asia/Kolkata")!
        f.timeZone = cal.timeZone
        return cal.date(from: comps).map(f.string) ?? id
    }
}

/// The month's totals from the GSTR-1 style report. Zyvor prepares this to file from; it does not file it.
public struct GSTSummary: Codable, Equatable, Sendable {
    public let period: String
    public let gstin: String?
    public let taxableMinor: Int64, cgstMinor: Int64, sgstMinor: Int64, igstMinor: Int64
    public let b2bInvoices: Int
    public let creditNotes: Int

    public var taxMinor: Int64 { cgstMinor + sgstMinor + igstMinor }
    public var isEmpty: Bool { taxableMinor == 0 && taxMinor == 0 && b2bInvoices == 0 && creditNotes == 0 }

    public init(period: String, gstin: String? = nil, taxableMinor: Int64 = 0, cgstMinor: Int64 = 0, sgstMinor: Int64 = 0, igstMinor: Int64 = 0, b2bInvoices: Int = 0, creditNotes: Int = 0) {
        self.period = period; self.gstin = gstin; self.taxableMinor = taxableMinor; self.cgstMinor = cgstMinor; self.sgstMinor = sgstMinor; self.igstMinor = igstMinor
        self.b2bInvoices = b2bInvoices; self.creditNotes = creditNotes
    }

    public init(from decoder: Decoder) throws {
        struct Totals: Decodable { let taxable_minor: Int64?, cgst_minor: Int64?, sgst_minor: Int64?, igst_minor: Int64? }
        struct Group: Decodable { let invoices: [Ignored]? }
        struct Ignored: Decodable {}
        enum K: String, CodingKey { case period, gstin, totals, b2b, cdnr, cdnur }
        let c = try decoder.container(keyedBy: K.self)
        let t = try c.decodeIfPresent(Totals.self, forKey: .totals)
        period = try c.decode(String.self, forKey: .period); gstin = try c.decodeIfPresent(String.self, forKey: .gstin)
        taxableMinor = t?.taxable_minor ?? 0; cgstMinor = t?.cgst_minor ?? 0; sgstMinor = t?.sgst_minor ?? 0; igstMinor = t?.igst_minor ?? 0
        b2bInvoices = (try c.decodeIfPresent([Group].self, forKey: .b2b) ?? []).reduce(0) { $0 + ($1.invoices?.count ?? 0) }
        struct Counted: Decodable {}
        creditNotes = (try c.decodeIfPresent([Counted].self, forKey: .cdnr) ?? []).count + (try c.decodeIfPresent([Counted].self, forKey: .cdnur) ?? []).count
    }
    public func encode(to encoder: Encoder) throws { var c = encoder.singleValueContainer(); try c.encode(period) }   // only decoded from the server
}

extension VayoraClient {
    public func gstSummary(period: GSTPeriod) async throws -> GSTSummary {
        try await send(request("GET", "/gst/gstr1?period=\(period.id)"), as: GSTSummary.self)
    }
}
