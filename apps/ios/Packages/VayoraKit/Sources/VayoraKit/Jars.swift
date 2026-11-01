import Foundation

/// A goal you set test money aside for. Money moves between the wallet and a jar and is never created or lost.
public struct Jar: Codable, Equatable, Identifiable, Sendable {
    public var id = UUID()
    public var name: String
    public var targetMinor: Int64
    public var savedMinor: Int64 = 0
    public init(name: String, targetMinor: Int64) { self.name = name; self.targetMinor = targetMinor }
    public var fraction: Double { targetMinor > 0 ? min(1, Double(savedMinor) / Double(targetMinor)) : 0 }
    public var reached: Bool { savedMinor >= targetMinor }
}

public struct JarBook: Codable, Equatable, Sendable {
    public enum Failure: Error, Equatable, Sendable { case badName, badTarget, tooMany, badAmount, unknownJar, walletTooLow, jarTooLow }
    public static let maxJars = 10
    public private(set) var jars: [Jar] = []
    public init() {}

    public var totalSavedMinor: Int64 { jars.reduce(0) { $0 + $1.savedMinor } }

    @discardableResult
    public mutating func add(name: String, targetMinor: Int64) throws -> Jar {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty, n.count <= 30 else { throw Failure.badName }
        guard targetMinor > 0, targetMinor <= 100_000_00_00 else { throw Failure.badTarget }
        guard jars.count < Self.maxJars else { throw Failure.tooMany }
        let j = Jar(name: n, targetMinor: targetMinor); jars.append(j); return j
    }

    /// Wallet to jar. The wallet cannot go below zero.
    public mutating func deposit(_ id: UUID, _ amountMinor: Int64, wallet: inout Int64) throws {
        guard amountMinor > 0 else { throw Failure.badAmount }
        guard let i = jars.firstIndex(where: { $0.id == id }) else { throw Failure.unknownJar }
        guard amountMinor <= wallet else { throw Failure.walletTooLow }
        wallet -= amountMinor; jars[i].savedMinor += amountMinor
    }

    /// Jar to wallet. A jar cannot go below zero.
    public mutating func withdraw(_ id: UUID, _ amountMinor: Int64, wallet: inout Int64) throws {
        guard amountMinor > 0 else { throw Failure.badAmount }
        guard let i = jars.firstIndex(where: { $0.id == id }) else { throw Failure.unknownJar }
        guard amountMinor <= jars[i].savedMinor else { throw Failure.jarTooLow }
        jars[i].savedMinor -= amountMinor; wallet += amountMinor
    }

    /// Delete a jar and give what it held back to the wallet.
    public mutating func remove(_ id: UUID, wallet: inout Int64) {
        guard let i = jars.firstIndex(where: { $0.id == id }) else { return }
        wallet += jars[i].savedMinor; jars.remove(at: i)
    }
}
