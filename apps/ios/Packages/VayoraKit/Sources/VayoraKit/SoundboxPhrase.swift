import Foundation

/// What a counter speaker says when a payment arrives. Numerals, not number words: the phone's own English and Hindi voices
/// read digits in their language, so there is no hand-made word table to get wrong.
public enum SoundboxPhrase {
    public enum Language: String, CaseIterable, Sendable { case english = "en-IN", hindi = "hi-IN"
        public var title: String { self == .english ? "English" : "हिन्दी" }
    }

    public static func received(_ amount: Money, language: Language) -> String {
        let rupees = amount.minor / 100, paise = amount.minor % 100
        switch language {
        case .english:
            var s = "\(rupees) \(rupees == 1 ? "rupee" : "rupees")"
            if paise > 0 { s += " and \(paise) paise" }
            return s + " received"
        case .hindi:
            var s = "\(rupees) रुपये"
            if paise > 0 { s += " \(paise) पैसे" }
            return s + " प्राप्त हुए"
        }
    }
}
