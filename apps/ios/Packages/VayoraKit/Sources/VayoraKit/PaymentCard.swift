import Foundation

/// What a shareable payment card says. The payee's UPI ID is left off on purpose: the card is for showing a friend, not for exposing an account.
public struct PaymentCardModel: Equatable, Sendable {
    public let amount: String
    public let headline: String
    public let to: String
    public let note: String?
    public let when: String
    public let paid: Bool

    public init(amount: Money, payeeName: String, payeeVPA: String, note: String, date: Date, paid: Bool, locale: Locale = Locale(identifier: "en_IN"), timeZone: TimeZone = .current) {
        self.amount = amount.display
        self.paid = paid
        let name = payeeName.trimmingCharacters(in: .whitespaces)
        self.to = name.isEmpty ? Self.masked(payeeVPA) : name
        self.headline = paid ? "Paid" : "Didn't go through"
        let n = note.trimmingCharacters(in: .whitespacesAndNewlines)
        self.note = n.isEmpty ? nil : String(n.prefix(60))
        let f = DateFormatter(); f.locale = locale; f.timeZone = timeZone; f.dateStyle = .medium; f.timeStyle = .short
        self.when = f.string(from: date)
    }

    /// "asha@okbank" becomes "as•••@okbank": enough to recognise, not enough to reuse.
    public static func masked(_ vpa: String) -> String {
        guard let at = vpa.firstIndex(of: "@") else { return "a payee" }
        let local = vpa[..<at]
        let keep = String(local.prefix(2))
        return keep + "•••" + vpa[at...]
    }
}
