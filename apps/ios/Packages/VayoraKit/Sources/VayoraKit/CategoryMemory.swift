import Foundation

/// What the person told us a payee is. It beats the word-guess in `SpendCategory.guess`, for every later payment to that UPI ID.
public struct CategoryMemory: Codable, Equatable, Sendable {
    public private(set) var byPayee: [String: String] = [:]
    public init() {}

    public mutating func set(_ vpa: String, to category: SpendCategory) { byPayee[vpa.lowercased()] = category.rawValue }
    public mutating func forget(_ vpa: String) { byPayee[vpa.lowercased()] = nil }
    public func category(for vpa: String) -> SpendCategory? { byPayee[vpa.lowercased()].flatMap(SpendCategory.init(rawValue:)) }

    /// The remembered category when there is one, otherwise the guess.
    public func resolve(vpa: String, name: String, note: String, transfer: Bool) -> SpendCategory {
        category(for: vpa) ?? SpendCategory.guess(name: name, note: note, transfer: transfer)
    }
}
