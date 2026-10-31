import Foundation

/// A payment request shown as a QR so another phone can pay it: `vayora://request?pa=…&pn=…&am=499.50&tn=…`.
/// It is read with the same rules as a scanned `upi://pay` link, and the amount is required (a request for "something" helps nobody).
public enum RequestPayload {
    public enum Failure: Error, Equatable { case notRequest, missingAmount, invalid(UPIPayload.Failure) }

    public static func make(payee: String, name: String, amount: Money, note: String) -> String {
        var c = URLComponents()
        c.scheme = "vayora"; c.host = "request"
        c.queryItems = [.init(name: "pa", value: payee), .init(name: "pn", value: name),
                        .init(name: "am", value: amount.decimalString), .init(name: "tn", value: note)]
        return c.string ?? ""
    }

    /// The request as an ordinary payment, so it goes through the same review, slide and PIN as any other.
    public static func parse(_ text: String) throws -> UPIPayload {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let c = URLComponents(string: t), c.scheme?.lowercased() == "vayora", c.host?.lowercased() == "request" else { throw Failure.notRequest }
        var q = URLComponents(); q.scheme = "upi"; q.host = "pay"
        q.queryItems = (c.queryItems ?? []) + [.init(name: "cu", value: "INR")]
        guard let asUPI = q.string else { throw Failure.notRequest }
        do {
            let p = try UPIPayload.parse(asUPI)
            guard p.amount != nil else { throw Failure.missingAmount }
            return p
        } catch let f as UPIPayload.Failure { throw Failure.invalid(f) }
    }
}
