import Foundation

/// When the app should ask for Face ID or the passcode again.
public enum LockPolicy {
    /// Lock on launch (no background time yet) and after the app was away for at least `timeoutMinutes` (0 = every time it leaves).
    public static func shouldLock(enabled: Bool, backgroundedAt: Date?, now: Date, timeoutMinutes: Int) -> Bool {
        guard enabled else { return false }
        guard let t = backgroundedAt else { return true }
        return now.timeIntervalSince(t) >= Double(max(0, timeoutMinutes)) * 60
    }
}

/// A plain warning for the oldest trick in UPI: someone says scanning or paying will bring you money.
public enum ScamHints {
    static let words = ["refund", "cashback", "reward", "prize", "lottery", "kyc", "claim", "receive money", "get money", "winning", "gift card", "verify"]

    /// Matches the payee name and the note, ignoring case. Returns the warning to show, or nil.
    public static func check(name: String, note: String) -> String? {
        let t = (name + " " + note).lowercased()
        guard words.contains(where: t.contains) else { return nil }
        return "Paying never brings money to you. If someone asked you to scan or pay to get a refund, prize or cashback, it is a scam."
    }
}

/// A file of the phone's own data (payees, history, jars, notes and so on) that can be saved and read back. Never the PIN or the server.
public enum Backup {
    public enum Failure: Error, Equatable, Sendable { case tooBig, notBackup, wrongVersion, badValue(String) }
    public static let version = 1
    public static let maxBytes = 5_000_000
    /// Collections and settings that are the person's own data, and the JSON shape each must have.
    public enum Shape: Sendable { case collection, text, flag, number }
    public static let keys: [String: Shape] = [
        "payees": .collection, "history": .collection, "reminders": .collection, "scheduled": .collection, "splits": .collection, "budgets": .collection,
        "categoryMemory": .collection, "jars": .collection, "vault": .collection, "autoPays": .collection,
        "myName": .text, "smallPayments": .flag, "balance": .number,
    ]

    /// `values` hold JSON objects (dictionaries, arrays, strings, numbers, booleans). Keys outside the list are dropped.
    public static func make(_ values: [String: Any], created: Date = Date()) throws -> Data {
        var data: [String: Any] = [:]
        for (k, v) in values where keys[k] != nil { data[k] = v }
        let obj: [String: Any] = ["app": "vayora-pay", "version": version, "created": ISO8601DateFormatter().string(from: created), "data": data]
        return try JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys, .prettyPrinted])
    }

    /// Only known keys, each with the right shape; anything else is refused or dropped. Nothing is applied here.
    public static func read(_ file: Data) throws -> [String: Any] {
        guard file.count <= maxBytes else { throw Failure.tooBig }
        guard let obj = try? JSONSerialization.jsonObject(with: file) as? [String: Any], obj["app"] as? String == "vayora-pay", let data = obj["data"] as? [String: Any] else { throw Failure.notBackup }
        guard obj["version"] as? Int == version else { throw Failure.wrongVersion }
        var out: [String: Any] = [:]
        for (k, v) in data {
            guard let shape = keys[k] else { continue }
            switch shape {
            case .collection: guard v is [Any] || v is [String: Any] else { throw Failure.badValue(k) }
            case .text: guard let s = v as? String, s.count <= 100 else { throw Failure.badValue(k) }
            case .flag: guard let n = v as? NSNumber, CFGetTypeID(n) == CFBooleanGetTypeID() else { throw Failure.badValue(k) }
            case .number: guard let n = v as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID(), n.int64Value >= 0, n.doubleValue < 1e15 else { throw Failure.badValue(k) }
            }
            out[k] = v
        }
        return out
    }
}
