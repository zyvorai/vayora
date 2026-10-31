import Foundation

/// A parsed `upi://pay?...` payload. Anything scanned from a QR code is hostile until proven otherwise.
public struct UPIPayload: Equatable, Sendable {
    public let payee: String       // pa
    public let name: String        // pn
    public let amount: Money?      // am (absent: the payer types it)
    public let note: String        // tn

    public enum Failure: Error, Equatable {
        case notUPI, missingPayee, badPayee, badAmount, unsupportedCurrency
    }

    public init(payee: String, name: String, amount: Money?, note: String) {
        self.payee = payee; self.name = name; self.amount = amount; self.note = note
    }

    public static func parse(_ text: String) throws -> UPIPayload {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.lowercased().hasPrefix("upi://pay?"),
              let comps = URLComponents(string: trimmed) else { throw Failure.notUPI }
        // A repeated parameter is ambiguous (which `am` is the real one?): refuse rather than pick.
        var q: [String: String] = [:]
        for item in comps.queryItems ?? [] {
            if q[item.name] != nil { throw Failure.badAmount }
            q[item.name] = item.value ?? ""
        }
        guard let pa = q["pa"], !pa.isEmpty else { throw Failure.missingPayee }
        guard isValidVPA(pa) else { throw Failure.badPayee }
        if let cu = q["cu"], cu.uppercased() != "INR" { throw Failure.unsupportedCurrency }
        var amount: Money?
        if let am = q["am"], !am.isEmpty {
            guard let m = Money(decimal: am) else { throw Failure.badAmount }
            amount = m
        }
        return UPIPayload(payee: pa, name: q["pn"] ?? "", amount: amount, note: q["tn"] ?? "")
    }

    /// handle@bank: letters, digits, `.`, `-`, `_` on both sides, one `@`, a sane length.
    public static func isValidVPA(_ v: String) -> Bool {
        let parts = v.split(separator: "@", omittingEmptySubsequences: false)
        guard parts.count == 2, v.count <= 80 else { return false }
        let ok: (Substring) -> Bool = { s in
            !s.isEmpty && s.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || ".-_".contains($0)) }
        }
        return ok(parts[0]) && ok(parts[1])
    }
}
