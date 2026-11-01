import Foundation

/// A small simulated bank that lives inside the app, so anyone can try Vayora with no server and no setup.
/// It answers the same requests the real test server does (the subset the apps use), through `DemoURLProtocol`,
/// so the app code that runs is the same code. Test money only; state lives in memory and resets when the app closes.
public final class DemoBank: @unchecked Sendable {
    public static let host = "demo.vayora.invalid"
    public static let shared = DemoBank()

    private let lock = NSLock()
    private let now: () -> Date
    /// Seconds after a QR is issued before the pretend customer "pays" it, the next time the app asks.
    public let customerPaysAfter: TimeInterval

    private struct Intent { var id: String; var amount: Int64; var currency: String; var note: String?; var created: Date; var attempts: [String] }
    private struct Attempt { var id: String; var intent: String; var amount: Int64; var created: Date; var status: String; var refunded: Int64; var uri: String }
    private struct Circle { var id: String, primary: String, delegate: String; var perPayment: Int64, total: Int64; var status: String; var spends: [(String, Int64, Date)] }

    private var intents: [String: Intent] = [:]
    private var order: [String] = []
    private var attempts: [String: Attempt] = [:]
    private var circles: [String: Circle] = [:]
    private var counter = 0

    public init(now: @escaping () -> Date = Date.init, customerPaysAfter: TimeInterval = 4) {
        self.now = now; self.customerPaysAfter = customerPaysAfter
    }

    private func nextID(_ p: String) -> String { counter += 1; return "\(p)_demo\(String(format: "%05d", counter))" }

    private static func iso(_ d: Date) -> String { ISO8601DateFormatter().string(from: d) }
    private static func err(_ status: Int, _ code: String, _ message: String) -> (Int, Data) {
        (status, (try? JSONSerialization.data(withJSONObject: ["error": ["code": code, "message": message]])) ?? Data())
    }
    private static func ok(_ obj: Any) -> (Int, Data) { (200, (try? JSONSerialization.data(withJSONObject: obj)) ?? Data()) }

    private func status(of i: Intent) -> String {
        let paid = i.attempts.contains { attempts[$0]?.status == "captured" }
        return paid ? "succeeded" : "requires_payment_method"
    }
    private func json(_ i: Intent) -> [String: Any] {
        ["id": i.id, "amount_minor": i.amount, "currency": i.currency, "status": status(of: i), "client_secret": "demo_secret_\(i.id)", "created_at": Self.iso(i.created)]
    }
    private func json(_ a: Attempt) -> [String: Any] {
        var d: [String: Any] = ["id": a.id, "status": a.status, "amount_minor": a.amount, "refunded_minor": a.refunded, "reserved_refund_minor": 0]
        d["client_action"] = ["type": "upi_intent", "uri": a.uri]
        return d
    }
    private func json(_ c: Circle) -> [String: Any] {
        let used = c.spends.reduce(Int64(0)) { $0 + $1.1 }
        return ["circle_id": c.id, "primary": c.primary, "delegate": c.delegate, "status": c.status,
                "max_per_payment_minor": c.perPayment, "total_limit_minor": c.total, "used_minor": used, "remaining_minor": max(0, c.total - used),
                "spends": c.spends.map { ["attempt_id": $0.0, "amount_minor": $0.1, "at": Self.iso($0.2)] as [String: Any] }]
    }

    /// The paise in a `upi://` amount like "250.00". Nil for anything that is not a positive amount with at most two decimals.
    public static func minor(fromUPIAmount s: String?) -> Int64? {
        guard let s, !s.isEmpty else { return nil }
        let parts = s.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count <= 2, let whole = Int64(parts[0]), whole >= 0 else { return nil }
        var frac: Int64 = 0
        if parts.count == 2 {
            guard parts[1].count <= 2, parts[1].allSatisfy(\.isNumber) else { return nil }
            frac = Int64(parts[1].padding(toLength: 2, withPad: "0", startingAt: 0)) ?? 0
        }
        let (m, o) = whole.multipliedReportingOverflow(by: 100)
        guard !o else { return nil }
        let total = m + frac
        return total > 0 ? total : nil
    }

    private static func query(_ uri: String) -> [String: String] {
        var out: [String: String] = [:]
        for q in URLComponents(string: uri)?.queryItems ?? [] { out[q.name] = q.value ?? "" }
        return out
    }

    /// Handles one request. `path` includes the `/v1` prefix, as the client sends it.
    public func handle(method: String, path: String, body: Data?) -> (status: Int, data: Data) {
        lock.lock(); defer { lock.unlock() }
        let obj = (body.flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]) ?? [:]
        let segs = path.split(separator: "/").map(String.init)
        guard segs.first == "v1" else { return Self.err(404, "not_found", "Unknown path") }
        let s = Array(segs.dropFirst())
        switch (method, s.count, s.first ?? "") {
        case ("POST", 1, "payment_intents"):
            guard let amt = (obj["amount_minor"] as? NSNumber)?.int64Value, amt > 0 else { return Self.err(422, "invalid_amount", "Enter an amount above zero.") }
            let id = nextID("pi")
            let i = Intent(id: id, amount: amt, currency: (obj["currency"] as? String) ?? "INR", note: obj["description"] as? String, created: now(), attempts: [])
            intents[id] = i; order.append(id)
            return Self.ok(json(i))
        case ("GET", 1, "payment_intents"):
            return Self.ok(["data": order.reversed().compactMap { intents[$0] }.map(json)])
        case ("GET", 2, "payment_intents"):
            guard let i = intents[s[1]] else { return Self.err(404, "not_found", "No such payment") }
            return Self.ok(["intent": json(i), "attempts": i.attempts.compactMap { attempts[$0] }.map(json)])
        case ("POST", 3, "payment_intents") where s[2] == "attempts":
            guard var i = intents[s[1]] else { return Self.err(404, "not_found", "No such payment") }
            let id = nextID("pa")
            var c = URLComponents(string: "upi://pay")!
            c.queryItems = [.init(name: "pa", value: "demo-shop@vayora"), .init(name: "pn", value: "Demo Shop"),
                            .init(name: "am", value: Money(minor: i.amount).decimalString), .init(name: "cu", value: i.currency),
                            .init(name: "tn", value: i.note ?? "Demo payment"), .init(name: "tr", value: id)]
            let a = Attempt(id: id, intent: i.id, amount: i.amount, created: now(), status: "pending", refunded: 0, uri: c.string ?? "")
            attempts[id] = a; i.attempts.append(id); intents[i.id] = i
            return Self.ok(json(a))
        case ("POST", 3, "payment_intents") where s[2] == "refunds":
            guard let i = intents[s[1]], let aid = i.attempts.first(where: { attempts[$0]?.status == "captured" }), var a = attempts[aid] else { return Self.err(409, "not_captured", "Only a paid payment can be refunded.") }
            let want = (obj["amount_minor"] as? NSNumber)?.int64Value ?? (a.amount - a.refunded)
            guard want > 0, want <= a.amount - a.refunded else { return Self.err(422, "refund_exceeds", "That is more than is left to refund.") }
            a.refunded += want; attempts[aid] = a
            return Self.ok(["id": nextID("re"), "status": "succeeded", "amount_minor": want])
        case ("POST", 3, "payment_attempts") where s[2] == "refresh":
            guard var a = attempts[s[1]] else { return Self.err(404, "not_found", "No such attempt") }
            if a.status == "pending", now().timeIntervalSince(a.created) >= customerPaysAfter { a.status = "captured"; attempts[a.id] = a }
            return Self.ok(json(a))
        case ("POST", 2, "sim") where s[1] == "pay":
            let uri = (obj["upi_uri"] as? String) ?? ""
            let q = Self.query(uri)
            guard uri.lowercased().hasPrefix("upi://"), Self.minor(fromUPIAmount: q["am"]) != nil else { return Self.err(422, "bad_uri", "That is not a payment link.") }
            let approve = (obj["approve"] as? Bool) ?? true
            // A pretend customer paying a code from this demo's own merchant side settles that payment.
            if approve, let tr = q["tr"], var a = attempts[tr], a.status == "pending" { a.status = "captured"; attempts[tr] = a }
            return Self.ok(["status": approve ? "captured" : "failed"])
        case ("POST", 2, "sim") where s[1] == "circle":
            return createCircle(obj)
        case ("POST", 3, "sim") where s[1] == "circle" && s[2] == "revoke":
            guard let id = obj["circle_id"] as? String, var c = circles[id] else { return Self.err(404, "not_found", "No such circle") }
            c.status = "revoked"; circles[id] = c
            return Self.ok(["status": "revoked"])
        case ("POST", 3, "sim") where s[1] == "circle" && s[2] == "pay":
            guard let id = obj["circle_id"] as? String, var c = circles[id] else { return Self.err(404, "not_found", "No such circle") }
            guard c.status == "active" else { return Self.err(409, "circle_revoked", "That circle has been taken back.") }
            guard let amt = Self.minor(fromUPIAmount: Self.query((obj["upi_uri"] as? String) ?? "")["am"]) else { return Self.err(422, "bad_uri", "That is not a payment link.") }
            let used = c.spends.reduce(Int64(0)) { $0 + $1.1 }
            guard amt <= c.perPayment else { return Self.err(422, "over_per_payment", "That is above the limit for one payment.") }
            guard amt <= c.total - used else { return Self.err(422, "over_total", "That is more than the circle has left.") }
            c.spends.append((nextID("ca"), amt, now())); circles[id] = c
            return Self.ok(["status": "captured", "remaining_minor": c.total - used - amt])
        case ("GET", 3, "sim") where s[1] == "circle":
            guard let c = circles[s[2]] else { return Self.err(404, "not_found", "No such circle") }
            return Self.ok(json(c))
        default:
            return Self.err(404, "not_in_demo", "That isn't part of the built-in demo. Connect a test server for it.")
        }
    }

    private func createCircle(_ obj: [String: Any]) -> (Int, Data) {
        guard let per = (obj["max_per_payment_minor"] as? NSNumber)?.int64Value, let total = (obj["total_limit_minor"] as? NSNumber)?.int64Value, per > 0, total >= per else {
            return Self.err(422, "invalid_limits", "The total must be at least the limit for one payment.")
        }
        let id = nextID("ci")
        circles[id] = Circle(id: id, primary: (obj["primary"] as? String) ?? "Me", delegate: (obj["delegate"] as? String) ?? "Friend", perPayment: per, total: total, status: "active", spends: [])
        return Self.ok(["circle_id": id])
    }
}
