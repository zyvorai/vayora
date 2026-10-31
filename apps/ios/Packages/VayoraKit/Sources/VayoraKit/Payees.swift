import Foundation

/// Someone you pay. `vpa` is the UPI ID the payment goes to; a phone number is kept as `<number>@zyvor-test`
/// (test mode has no bank to look a number up in).
public struct Payee: Codable, Equatable, Hashable, Identifiable, Sendable {
    public var id: String { vpa }
    public let vpa: String
    public var name: String
    public var favourite: Bool
    public var lastPaid: Date?
    public init(vpa: String, name: String, favourite: Bool = false, lastPaid: Date? = nil) {
        self.vpa = vpa; self.name = name; self.favourite = favourite; self.lastPaid = lastPaid
    }
}

public enum PayeeInput {
    public static let phoneDomain = "zyvor-test"

    /// A UPI ID, or an Indian mobile number (10 digits starting 6 to 9, optionally `+91` or a leading `0`). Anything else is nil.
    public static func vpa(from input: String) -> String? {
        let t = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.contains("@") { return UPIPayload.isValidVPA(t) ? t.lowercased() : nil }
        var d = t.filter { !" -()".contains($0) }
        if d.hasPrefix("+91") { d.removeFirst(3) } else if d.hasPrefix("0") { d.removeFirst() }
        guard d.count == 10, d.allSatisfy({ $0.isASCII && $0.isNumber }), "6789".contains(d.first!) else { return nil }
        return "\(d)@\(phoneDomain)"
    }
}

/// The saved payees: no duplicates, favourites first, then the most recently paid.
public struct PayeeBook: Codable, Equatable, Sendable {
    public private(set) var all: [Payee] = []
    public init(_ all: [Payee] = []) { self.all = all }

    public var sorted: [Payee] {
        all.sorted {
            if $0.favourite != $1.favourite { return $0.favourite }
            if ($0.lastPaid ?? .distantPast) != ($1.lastPaid ?? .distantPast) { return ($0.lastPaid ?? .distantPast) > ($1.lastPaid ?? .distantPast) }
            return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
        }
    }

    /// Adds or updates by UPI ID. Returns false when the input is not a UPI ID or phone number.
    @discardableResult
    public mutating func save(name: String, input: String, paidAt: Date? = nil) -> Bool {
        guard let vpa = PayeeInput.vpa(from: input) else { return false }
        let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
        if let i = all.firstIndex(where: { $0.vpa == vpa }) {
            if !label.isEmpty { all[i].name = label }
            if let paidAt { all[i].lastPaid = paidAt }
        } else {
            all.append(Payee(vpa: vpa, name: label.isEmpty ? vpa : label, lastPaid: paidAt))
        }
        return true
    }
    public mutating func toggleFavourite(_ vpa: String) { if let i = all.firstIndex(where: { $0.vpa == vpa }) { all[i].favourite.toggle() } }
    public mutating func remove(_ vpa: String) { all.removeAll { $0.vpa == vpa } }
}
