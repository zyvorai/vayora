import Foundation

public struct PaymentCSVRow: Equatable, Sendable {
    public let date: Date, id: String, amountMinor: Int64, status: String, refundedMinor: Int64
    public init(date: Date, id: String, amountMinor: Int64, status: String, refundedMinor: Int64) { self.date = date; self.id = id; self.amountMinor = amountMinor; self.status = status; self.refundedMinor = refundedMinor }
    public var paid: Bool { status == "succeeded" }
    public var netMinor: Int64 { paid ? max(0, amountMinor - refundedMinor) : 0 }
}

/// What an accountant asks for: every payment, and a total per day. Days are Indian days (IST), whatever the phone's time zone is.
public enum PaymentsCSV {
    public static var ist: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; return c }

    static func cell(_ s: String) -> String {
        var v = s.replacingOccurrences(of: "\r", with: " ").replacingOccurrences(of: "\n", with: " ")
        if let f = v.first, "=+-@\t".contains(f) { v = "'" + v }
        return (v.contains(",") || v.contains("\"")) ? "\"" + v.replacingOccurrences(of: "\"", with: "\"\"") + "\"" : v
    }
    static func day(_ d: Date, _ c: Calendar) -> String { let x = c.dateComponents([.year, .month, .day], from: d); return String(format: "%04d-%02d-%02d", x.year ?? 0, x.month ?? 0, x.day ?? 0) }

    public static func payments(_ rows: [PaymentCSVRow], calendar: Calendar = ist) -> String {
        var out = "date_ist,time_ist,payment_id,amount_inr,status,refunded_inr,net_inr\n"
        for r in rows.sorted(by: { $0.date < $1.date }) {
            let t = calendar.dateComponents([.hour, .minute], from: r.date)
            out += [day(r.date, calendar), String(format: "%02d:%02d", t.hour ?? 0, t.minute ?? 0), cell(r.id), Money(minor: r.amountMinor).decimalString, cell(r.status),
                    Money(minor: r.refundedMinor).decimalString, Money(minor: r.netMinor).decimalString].joined(separator: ",") + "\n"
        }
        return out
    }

    /// Paid payments per day: how many, the gross, what was given back, and the net. Unpaid payments are not counted.
    public static func daily(_ rows: [PaymentCSVRow], calendar: Calendar = ist) -> String {
        var days: [String: (n: Int, gross: Int64, back: Int64)] = [:]
        for r in rows where r.paid { let k = day(r.date, calendar); var e = days[k] ?? (0, 0, 0); e.n += 1; e.gross += r.amountMinor; e.back += min(r.refundedMinor, r.amountMinor); days[k] = e }
        var out = "date_ist,payments,gross_inr,refunded_inr,net_inr\n"
        for k in days.keys.sorted() { let e = days[k]!; out += "\(k),\(e.n),\(Money(minor: e.gross).decimalString),\(Money(minor: e.back).decimalString),\(Money(minor: e.gross - e.back).decimalString)\n" }
        return out
    }
}
