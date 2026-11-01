import Foundation

/// "Pay Asha 200 rupees", "send Ravi 50.50", "आशा को 200 रुपये भेजो", "Ravi ko 300 rupaye bhejo": spoken words turned into a payment to review.
/// It only ever proposes a payment to someone already saved; the person still sees the review and slides and enters the PIN.
public enum VoiceCommand {
    public enum Result: Equatable, Sendable {
        case pay(payee: Payee, amount: Money)
        case ambiguous([Payee], Money)
        case noPayee(String)
        case noAmount
        case notAPayment
    }

    static let verbs: Set<String> = ["pay", "send", "transfer", "give", "भेजो", "भेजें", "भेज", "दो", "bhejo", "bhej", "do", "दे", "de"]
    static let fillers: Set<String> = ["rupees", "rupee", "rs", "inr", "₹", "रुपये", "रुपया", "रुपए", "rupaye", "rupay", "to", "को", "ko", "please", "the", "के", "ke", "लिए", "liye", "for", "and", "paise", "पैसे"]

    /// Devanagari digits become ASCII so "२००" is 200.
    static func ascii(_ s: String) -> String {
        let map: [Character: Character] = ["०": "0", "१": "1", "२": "2", "३": "3", "४": "4", "५": "5", "६": "6", "७": "7", "८": "8", "९": "9"]
        return String(s.map { map[$0] ?? $0 })
    }

    public static func parse(_ spoken: String, payees: [Payee]) -> Result {
        let text = ascii(spoken).lowercased().replacingOccurrences(of: ",", with: "").replacingOccurrences(of: "₹", with: " ₹ ")
        let words = text.split(whereSeparator: { !$0.isLetter && !$0.isNumber && $0 != "." && $0 != "₹" && !$0.unicodeScalars.contains(where: { (0x0900...0x097F).contains($0.value) }) }).map(String.init)
        guard words.contains(where: { verbs.contains($0) }) else { return .notAPayment }

        // the amount: the first plain number
        var amount: Money?; var amountAt: Int?
        for (i, w) in words.enumerated() where w.first?.isNumber == true && w.allSatisfy({ $0.isNumber || $0 == "." }) {
            if let m = Money(decimal: w), m.minor > 0, m.minor <= 100_000_00_00 { amount = m; amountAt = i; break }
        }
        guard let amt = amount, let at = amountAt else { return .noAmount }

        // the name: whatever is left once verbs, fillers and the amount are removed
        let nameWords = words.enumerated().filter { $0.offset != at && !verbs.contains($0.element) && !fillers.contains($0.element) }.map(\.element)
        let name = nameWords.joined(separator: " ")
        guard !name.isEmpty else { return .noPayee("") }

        let hits = payees.filter { p in
            let n = p.name.lowercased()
            return n == name || n.contains(name) || nameWords.allSatisfy { n.contains($0) } || name.contains(n)
        }
        let exact = hits.filter { $0.name.lowercased() == name }
        if exact.count == 1 { return .pay(payee: exact[0], amount: amt) }
        if hits.count == 1 { return .pay(payee: hits[0], amount: amt) }
        if hits.isEmpty { return .noPayee(name) }
        return .ambiguous(hits, amt)
    }
}
