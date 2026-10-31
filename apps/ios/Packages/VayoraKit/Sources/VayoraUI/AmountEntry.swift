import Foundation
import VayoraKit

/// What the counter keypad has typed so far. Text, never a Double: "49.5" stays "49.5" until it is turned into paise.
public struct AmountEntry: Equatable, Sendable {
    public private(set) var text = ""
    public init(_ text: String = "") { self.text = text }
    static let maxIntegerDigits = 7

    public mutating func digit(_ d: Int) {
        guard (0...9).contains(d) else { return }
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        if parts.count == 2 { guard parts[1].count < 2 else { return } }
        else {
            if text == "0" { text = String(d); return }      // no leading zeros: "05" becomes "5"
            guard text.count < Self.maxIntegerDigits else { return }
        }
        text += String(d)
    }
    public mutating func point() {
        guard !text.contains(".") else { return }
        text = text.isEmpty ? "0." : text + "."
    }
    public mutating func delete() { _ = text.popLast() }
    public mutating func clear() { text = "" }
    /// Nil until it is a valid, positive amount.
    public var money: Money? { Money(decimal: text) }
}
