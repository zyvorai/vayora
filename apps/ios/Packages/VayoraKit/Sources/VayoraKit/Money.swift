import Foundation

/// An amount in integer minor units (paise). Never a floating point number: ₹499.00 is `49900`.
public struct Money: Equatable, Hashable, Comparable, Sendable {
    public let minor: Int64
    public let currency: String

    public init(minor: Int64, currency: String = "INR") {
        self.minor = minor
        self.currency = currency
    }

    public static func < (a: Money, b: Money) -> Bool { a.minor < b.minor }

    /// Parses what a person types or a UPI `am` field carries: "499", "499.5", "499.50". Rejects
    /// negatives, zero, more than two decimals, exponents and anything that is not plain digits.
    public init?(decimal text: String, currency: String = "INR") {
        let parts = text.split(separator: ".", maxSplits: 1, omittingEmptySubsequences: false)
        guard let whole = parts.first, !whole.isEmpty,
              whole.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
        var frac = parts.count == 2 ? String(parts[1]) : ""
        if parts.count == 2 && frac.isEmpty { return nil }
        guard frac.count <= 2, frac.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
        while frac.count < 2 { frac += "0" }
        guard let w = Int64(whole), w <= 10_000_000_000,
              let f = Int64(frac) else { return nil }
        let total = w * 100 + f
        guard total > 0 else { return nil }
        self.init(minor: total, currency: currency)
    }

    /// "499.00", for UPI payloads and APIs that want a decimal string.
    public var decimalString: String {
        String(format: "%lld.%02lld", minor / 100, minor % 100)
    }

    /// "₹499.00", with Indian digit grouping (1,23,456.00).
    public var display: String {
        let rupees = String(minor / 100)
        var grouped = ""
        for (i, ch) in rupees.reversed().enumerated() {
            let atGroupEdge = i == 3 || (i > 3 && (i - 3) % 2 == 0)
            if atGroupEdge { grouped.append(",") }
            grouped.append(ch)
        }
        let symbol = currency == "INR" ? "₹" : currency + " "
        return symbol + String(grouped.reversed()) + String(format: ".%02lld", minor % 100)
    }
}
