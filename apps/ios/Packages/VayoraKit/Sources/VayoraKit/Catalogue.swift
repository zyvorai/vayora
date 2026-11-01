import Foundation

/// Something a shop sells. Prices are integer paise. GST is shown as arithmetic on the bill and is never a tax invoice.
public struct Product: Codable, Equatable, Hashable, Identifiable, Sendable {
    public var id: UUID
    public var name: String
    public var priceMinor: Int64
    /// 0...100. Nil means the shop adds no GST line for this item.
    public var gstPercent: Int?
    /// True when `priceMinor` already includes the GST.
    public var gstInclusive: Bool

    public init(id: UUID = UUID(), name: String, priceMinor: Int64, gstPercent: Int? = nil, gstInclusive: Bool = false) {
        self.id = id; self.name = name; self.priceMinor = priceMinor; self.gstPercent = gstPercent; self.gstInclusive = gstInclusive
    }
}

public struct Catalogue: Codable, Equatable, Sendable {
    public private(set) var products: [Product] = []
    public static let maxProducts = 200
    public static let maxNameLength = 40
    public init() {}

    public enum Failure: Error, Equatable { case emptyName, nameTooLong, duplicate, badPrice, badGST, full }

    private func validate(_ p: Product, ignoring id: UUID?) throws {
        let n = p.name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else { throw Failure.emptyName }
        guard n.count <= Self.maxNameLength else { throw Failure.nameTooLong }
        guard p.priceMinor > 0, p.priceMinor <= 100_000_000 else { throw Failure.badPrice }
        if let g = p.gstPercent { guard (0...100).contains(g) else { throw Failure.badGST } }
        if products.contains(where: { $0.id != id && $0.name.caseInsensitiveCompare(n) == .orderedSame }) { throw Failure.duplicate }
    }

    public mutating func add(_ p: Product) throws {
        guard products.count < Self.maxProducts else { throw Failure.full }
        try validate(p, ignoring: nil)
        var q = p; q.name = p.name.trimmingCharacters(in: .whitespacesAndNewlines)
        products.append(q)
    }

    public mutating func update(_ p: Product) throws {
        guard let i = products.firstIndex(where: { $0.id == p.id }) else { return }
        try validate(p, ignoring: p.id)
        var q = p; q.name = p.name.trimmingCharacters(in: .whitespacesAndNewlines)
        products[i] = q
    }

    public mutating func remove(_ id: UUID) { products.removeAll { $0.id == id } }
}

/// One line of a bill: what was bought, with the price and GST as they were when it was added.
public struct BillLine: Codable, Equatable, Sendable {
    public let name: String
    public let quantity: Int
    public let unitMinor: Int64
    public let gstPercent: Int?
    public let gstInclusive: Bool

    /// Net before GST, GST, and what the customer pays for this line, in paise. GST rounds half up, once per line.
    public var parts: (net: Int64, gst: Int64, gross: Int64) {
        let amount = unitMinor * Int64(quantity)
        guard let g = gstPercent, g > 0 else { return (amount, 0, amount) }
        if gstInclusive {
            let d = Int64(100 + g)
            let net = (amount * 100 * 2 + d) / (2 * d)
            return (net, amount - net, amount)
        }
        let gst = (amount * Int64(g) * 2 + 100) / 200
        return (amount, gst, amount + gst)
    }
}

public struct Bill: Codable, Equatable, Sendable {
    public private(set) var lines: [BillLine] = []
    public init() {}

    public static let maxQuantity = 999

    public mutating func add(_ p: Product, quantity q: Int = 1) {
        guard q > 0 else { return }
        if let i = lines.firstIndex(where: { $0.name == p.name && $0.unitMinor == p.priceMinor && $0.gstPercent == p.gstPercent && $0.gstInclusive == p.gstInclusive }) {
            let n = min(Self.maxQuantity, lines[i].quantity + q)
            lines[i] = BillLine(name: p.name, quantity: n, unitMinor: p.priceMinor, gstPercent: p.gstPercent, gstInclusive: p.gstInclusive)
        } else {
            lines.append(BillLine(name: p.name, quantity: min(Self.maxQuantity, q), unitMinor: p.priceMinor, gstPercent: p.gstPercent, gstInclusive: p.gstInclusive))
        }
    }

    public mutating func remove(_ p: Product) {
        guard let i = lines.firstIndex(where: { $0.name == p.name && $0.unitMinor == p.priceMinor }) else { return }
        let l = lines[i]
        if l.quantity <= 1 { lines.remove(at: i) }
        else { lines[i] = BillLine(name: l.name, quantity: l.quantity - 1, unitMinor: l.unitMinor, gstPercent: l.gstPercent, gstInclusive: l.gstInclusive) }
    }

    public mutating func clear() { lines = [] }
    public var isEmpty: Bool { lines.isEmpty }
    public func quantity(of p: Product) -> Int { lines.first { $0.name == p.name && $0.unitMinor == p.priceMinor }?.quantity ?? 0 }

    public var netMinor: Int64 { lines.reduce(0) { $0 + $1.parts.net } }
    public var gstMinor: Int64 { lines.reduce(0) { $0 + $1.parts.gst } }
    public var totalMinor: Int64 { lines.reduce(0) { $0 + $1.parts.gross } }

    /// The payment note: "2 x Tea, 1 x Vada pav". Cut at `limit` characters with "+N more".
    public func note(limit: Int = 100) -> String {
        let items = lines.map { "\($0.quantity) x \($0.name)" }
        var out = ""
        for (i, s) in items.enumerated() {
            let next = out.isEmpty ? s : out + ", " + s
            if next.count > limit { return out + (out.isEmpty ? "" : " ") + "+\(items.count - i) more" }
            out = next
        }
        return out
    }
}

/// A bill that was turned into a payment, kept on the phone so sales can be grouped by item.
public struct BillRecord: Codable, Equatable, Sendable {
    public let intentID: String
    public let date: Date
    public let lines: [BillLine]
    public init(intentID: String, date: Date, lines: [BillLine]) { self.intentID = intentID; self.date = date; self.lines = lines }
}

public enum SalesByItem {
    public struct Row: Equatable, Sendable { public let name: String; public let quantity: Int; public let grossMinor: Int64 }

    /// Items sold in paid bills since `since`, most sold first (ties by name). Only bills built on this phone are known.
    public static func top(_ log: [BillRecord], paid: Set<String>, since: Date) -> [Row] {
        var q: [String: Int] = [:], m: [String: Int64] = [:]
        for r in log where r.date >= since && paid.contains(r.intentID) {
            for l in r.lines { q[l.name, default: 0] += l.quantity; m[l.name, default: 0] += l.parts.gross }
        }
        return q.map { Row(name: $0.key, quantity: $0.value, grossMinor: m[$0.key] ?? 0) }
            .sorted { $0.quantity != $1.quantity ? $0.quantity > $1.quantity : $0.name < $1.name }
    }

    public static func summary(_ rows: [Row], title: String) -> String {
        guard !rows.isEmpty else { return "\(title)\nNo itemised sales yet." }
        return ([title] + rows.prefix(10).map { "\($0.quantity) x \($0.name): \(Money(minor: $0.grossMinor).display)" } + ["Bills made in this app only. Not a tax invoice."]).joined(separator: "\n")
    }
}
