import Foundation

/// The owner's PIN for cashier mode, with a lockout after wrong tries. A convenience on a shared counter phone, not security:
/// the API key on the phone is still the whole shop's key.
public struct PinGate: Equatable, Sendable {
    public static let maxTries = 5
    public static let lockSeconds: TimeInterval = 30

    public private(set) var failures = 0
    public private(set) var lockedUntil: Date?
    public init() {}

    /// 4 to 6 digits.
    public static func isValidPIN(_ s: String) -> Bool { (4...6).contains(s.count) && s.allSatisfy { $0.isASCII && $0.isNumber } }

    public func isLocked(now: Date) -> Bool { lockedUntil.map { now < $0 } ?? false }
    public func secondsLeft(now: Date) -> Int { max(0, Int((lockedUntil?.timeIntervalSince(now) ?? 0).rounded(.up))) }

    /// Returns true when `entered` matches. Wrong tries count; the fifth locks for 30 seconds and a locked gate refuses even the right PIN.
    public mutating func attempt(_ entered: String, against pin: String, now: Date) -> Bool {
        if isLocked(now: now) { return false }
        if lockedUntil != nil { lockedUntil = nil; failures = 0 }
        if entered == pin { failures = 0; return true }
        failures += 1
        if failures >= Self.maxTries { lockedUntil = now.addingTimeInterval(Self.lockSeconds) }
        return false
    }
}

/// Names you give to payments, kept on the phone: a running total per name. Records only: nothing is lent, charged or sent.
public struct RegularsBook: Codable, Equatable, Sendable {
    public private(set) var tags: [String: String] = [:]   // payment intent id -> name
    public init() {}

    public static func clean(_ name: String) -> String { name.trimmingCharacters(in: .whitespacesAndNewlines).split(whereSeparator: \.isWhitespace).joined(separator: " ") }

    public mutating func tag(_ intentID: String, as name: String) {
        let n = Self.clean(name)
        if n.isEmpty { tags[intentID] = nil } else { tags[intentID] = String(n.prefix(40)) }
    }
    public func name(for intentID: String) -> String? { tags[intentID] }
    /// Names already used, for quick picking, most used first.
    public var names: [String] {
        Dictionary(grouping: tags.values, by: { $0.lowercased() }).values.sorted { $0.count != $1.count ? $0.count > $1.count : $0[0].lowercased() < $1[0].lowercased() }.map { $0[0] }
    }

    public struct Total: Equatable, Sendable { public let name: String, minor: Int64, count: Int }
    /// Paid money per name, biggest first. A name is matched ignoring case. Unpaid payments never count.
    public func totals(_ payments: [(id: String, minor: Int64, paid: Bool)]) -> [Total] {
        var sums: [String: (name: String, minor: Int64, count: Int)] = [:]
        for p in payments where p.paid {
            guard let n = tags[p.id] else { continue }
            let k = n.lowercased()
            var e = sums[k] ?? (n, 0, 0); e.minor += p.minor; e.count += 1; sums[k] = e
        }
        return sums.values.map { Total(name: $0.name, minor: $0.minor, count: $0.count) }.sorted { $0.minor != $1.minor ? $0.minor > $1.minor : $0.name < $1.name }
    }
}
