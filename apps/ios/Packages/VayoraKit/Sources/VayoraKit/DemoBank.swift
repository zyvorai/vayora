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
    private struct Link { var id: String; var amount: Int64; var note: String?; var maxUses: Int?; var expires: String?; var canceled: Bool; var created: Date }
    private var links: [String: Link] = [:]
    private var linkOrder: [String] = []
    private var payoutsToPay: [String: String] = ["po_demo2": "approved"]
    private var refundsFailed: [String: String] = ["re_demo2": "failed"]
    private var disputes: [String: (status: String, evidence: Int)] = ["dp_demo1": ("open", 0)]
    private var approvals: [String: String] = ["re_demo1": "pending_approval", "po_demo1": "pending_approval"]
    private var drafts: [(id: String, number: String, message: String, status: String)] = [
        ("rd_demo1", "INV-1001", "Hello, this is a reminder that invoice INV-1001 for Rs 1,250.00 is 12 days past due.", "pending"),
        ("rd_demo2", "INV-1002", "A gentle reminder that invoice INV-1002 is due this week.", "pending")]
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
    private func json(_ l: Link) -> [String: Any] {
        var d: [String: Any] = ["id": l.id, "path": "/pay/link/demo/\(l.id)", "amount_minor": l.amount, "currency": "INR", "uses": 0,
                                "status": l.canceled ? "canceled" : "active", "created_at": Self.iso(l.created), "can_email": false]
        if let n = l.note { d["description"] = n }
        if let m = l.maxUses { d["max_uses"] = m }
        if let e = l.expires { d["expires_at"] = e }
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
    public func handle(method: String, path: String, body: Data?, query: String? = nil) -> (status: Int, data: Data) {
        lock.lock(); defer { lock.unlock() }
        let obj = (body.flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]) ?? [:]
        let segs = path.split(separator: "/").map(String.init)
        guard segs.first == "v1" else { return Self.err(404, "not_found", "Unknown path") }
        let s = Array(segs.dropFirst())
        let wanted = Self.query("x://x?" + (query ?? ""))["status"]   // the list filter a client asked for, if any
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
        case ("GET", 3, "payment_intents") where s[2] == "timeline":
            guard let i = intents[s[1]] else { return Self.err(404, "not_found", "No such payment") }
            var events: [[String: Any]] = [["at": Self.iso(i.created), "kind": "intent", "title": "Payment of \(Money(minor: i.amount).decimalString) INR created"]]
            for aid in i.attempts { if let a = attempts[aid] {
                events.append(["at": Self.iso(a.created), "kind": "attempt", "title": "A payer was shown a UPI QR"])
                if a.status == "captured" { events.append(["at": Self.iso(a.created.addingTimeInterval(1)), "kind": "captured", "title": "The payer paid it"]) }
                if a.refunded > 0 { events.append(["at": Self.iso(a.created.addingTimeInterval(2)), "kind": "refund", "title": "\(Money(minor: a.refunded).decimalString) INR refunded"]) } } }
            return Self.ok(["payment_id": i.id, "status": status(of: i), "standing": status(of: i) == "succeeded" ? "Paid." : "Created, but nobody has paid yet.", "events": events])
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
        case ("GET", 1, "disputes"):
            return Self.ok(["data": disputes.map { ["id": $0.key, "amount_minor": 45000, "reason": "Customer says they were charged twice", "status": $0.value.status, "respond_by": "2026-11-10", "evidence_count": $0.value.evidence] as [String: Any] }])
        case ("POST", 3, "disputes") where s[2] == "evidence":
            guard let d = disputes[s[1]], d.status == "open" || d.status == "under_review" else { return Self.err(409, "not_open", "That dispute is closed.") }
            guard let n = obj["note"] as? String, !n.trimmingCharacters(in: .whitespaces).isEmpty else { return Self.err(422, "note_required", "Describe what this evidence shows.") }
            disputes[s[1]] = ("under_review", d.evidence + 1)
            return Self.ok(["id": s[1], "status": "under_review"])
        case ("POST", 3, "disputes") where s[2] == "resolve":
            guard let d = disputes[s[1]], d.status == "open" || d.status == "under_review" else { return Self.err(409, "not_open", "That dispute is closed.") }
            guard let o = obj["outcome"] as? String, o == "won" || o == "lost" else { return Self.err(422, "invalid_outcome", "outcome must be won or lost") }
            disputes[s[1]] = (o, d.evidence)
            return Self.ok(["id": s[1], "status": o])
        case ("GET", 2, "insights") where s[1].hasPrefix("cashflow"):
            return Self.ok(["as_of": "2026-11-03", "currency": "INR", "weeks": [["starts": "2026-11-03", "due_minor": 80000], ["starts": "2026-11-10", "due_minor": 0], ["starts": "2026-11-17", "due_minor": 0], ["starts": "2026-11-24", "due_minor": 0]],
                            "overdue_minor": 125000, "due_later_minor": 0, "no_due_date_minor": 0, "promised_minor": 60000, "payouts_waiting_minor": 1200000, "daily_receipts_run_rate_minor": 410000])
        case ("GET", 1, "settlements"):
            return Self.ok(["data": [["id": "st_demo1", "provider": "demo", "utr": "DEMOUTR0001", "gross_minor": 500000, "refunds_minor": 25000, "fee_minor": 11800, "adjustment_minor": 0, "net_minor": 463200, "settled_on": "2026-11-02", "status": "matched"] as [String: Any]]])
        case ("GET", 1, "payouts") where wanted == "approved":
            return Self.ok(["data": payoutsToPay.filter { $0.value == "approved" }.map { ["id": $0.key, "amount_minor": 350000, "currency": "INR", "status": "approved", "purpose": "Rent for November"] as [String: Any] }])
        case ("POST", 3, "payouts") where s[2] == "mark_paid":
            guard payoutsToPay[s[1]] == "approved" else { return Self.err(409, "not_approved", "That payout is not waiting to be paid.") }
            guard let ref = (obj["bank_ref"] as? String)?.trimmingCharacters(in: .whitespaces), (6...40).contains(ref.count) else { return Self.err(422, "invalid_bank_ref", "bank_ref (the UTR) must be 6-40 characters") }
            payoutsToPay[s[1]] = "paid"
            return Self.ok(["id": s[1], "status": "paid"])
        case ("GET", 1, "refunds") where wanted == "failed":
            return Self.ok(["data": refundsFailed.filter { $0.value == "failed" }.map { ["id": $0.key, "amount_minor": 12000, "currency": "INR", "status": "failed", "failure_reason": "The customer's bank did not accept it"] as [String: Any] }])
        case ("POST", 3, "refunds") where s[2] == "retry":
            guard refundsFailed[s[1]] == "failed" else { return Self.err(409, "not_failed", "Only a failed refund can be retried.") }
            refundsFailed[s[1]] = "processing"
            return Self.ok(["id": s[1], "status": "processing"])
        case ("GET", 2, "gst") where s[1] == "gstr1":
            return Self.ok(["note": "GSTR-1 style summary to prepare a return from; Zyvor does not file it", "period": Self.query("x://x?" + (query ?? ""))["period"] ?? "", "gstin": "29ABCDE1234F1Z5",
                            "b2b": [["buyer_gstin": "27ABCDE1234F1Z5", "invoices": [["number": "INV-1001"], ["number": "INV-1002"]]]], "b2cs": [], "cdnr": [["note_id": "cn_1"]], "cdnur": [], "hsn": [],
                            "totals": ["taxable_minor": 1_000_000, "cgst_minor": 90_000, "sgst_minor": 90_000, "igst_minor": 0]])
        case ("GET", 3, "settlements") where s[2] == "explain":
            guard s[1] == "st_demo1" else { return Self.err(404, "not_found", "No such settlement") }
            return Self.ok(["settlement": ["id": "st_demo1"], "payments": [["matched": true], ["matched": true]], "cases": [], "bank_match": NSNull(),
                            "arithmetic": ["balances": true], "expected_minor": 493200, "received_minor": 463200, "difference_minor": -30000,
                            "difference_breakdown": [["reason": "provider fees and taxes (no fee schedule configured)", "amount_minor": -11800], ["reason": "settlement adjustments", "amount_minor": -18200]],
                            "narrative": "Demo paid ₹4,632.00 on 2026-11-02: 2 payments worth ₹5,000.00, less ₹250.00 of refunds. It deducted ₹118.00 in fees and taxes (2.36% of payment value)."])
        case ("GET", 1, "refunds"):
            return Self.ok(["data": approvals["re_demo1"] == "pending_approval" ? [["id": "re_demo1", "amount_minor": 25000, "currency": "INR", "status": "pending_approval", "reason": "Customer returned the item", "requested_by": "cashier"] as [String: Any]] : []])
        case ("GET", 1, "payouts"):
            return Self.ok(["data": approvals["po_demo1"] == "pending_approval" ? [["id": "po_demo1", "amount_minor": 1200000, "currency": "INR", "status": "pending_approval", "purpose": "Supplier invoice 88", "requested_by": "accounts"] as [String: Any]] : []])
        case ("POST", 3, "refunds"), ("POST", 3, "payouts"):
            guard (s[2] == "approve" || s[2] == "reject"), approvals[s[1]] == "pending_approval" else { return Self.err(409, "not_pending_approval", "That is not waiting for approval.") }
            approvals[s[1]] = s[2] == "approve" ? "approved" : "rejected"
            return Self.ok(["id": s[1], "status": approvals[s[1]] ?? ""])
        case ("GET", 2, "collections") where s[1] == "ageing":
            func inv(_ id: String, _ n: String, _ due: String, _ amt: Int64, _ days: Int) -> [String: Any] { ["id": id, "number": n, "due_date": due, "outstanding_minor": amt, "days_past_due": days] }
            return Self.ok(["as_of": "2026-11-03", "data": [
                ["currency": "INR", "bucket": "1_30", "outstanding_minor": 125000, "count": 1, "invoices": [inv("in_demo1", "INV-1001", "2026-10-22", 125000, 12)]],
                ["currency": "INR", "bucket": "current", "outstanding_minor": 80000, "count": 1, "invoices": [inv("in_demo2", "INV-1002", "2026-11-08", 80000, 0)]]]])
        case ("GET", 2, "collections") where s[1] == "drafts":
            return Self.ok(["data": drafts.filter { $0.status == "pending" }.map { ["id": $0.id, "invoice_number": $0.number, "channel": "email", "message": $0.message] }])
        case ("POST", 4, "collections") where s[1] == "drafts" && (s[3] == "approve" || s[3] == "reject"):
            guard let i = drafts.firstIndex(where: { $0.id == s[2] && $0.status == "pending" }) else { return Self.err(404, "not_found", "No such draft.") }
            drafts[i].status = s[3] == "approve" ? "approved" : "rejected"
            return Self.ok(["id": s[2], "status": drafts[i].status])
        case ("POST", 3, "invoices") where s[2] == "send":
            return Self.err(409, "no_customer_email", "The built-in demo has no mail. Connect a test server to email an invoice.")
        case ("POST", 3, "invoices") where s[2] == "promises":
            return (201, (try? JSONSerialization.data(withJSONObject: ["id": nextID("pr")])) ?? Data())
        case ("POST", 1, "payment_links"):
            guard let amt = (obj["amount_minor"] as? NSNumber)?.int64Value, amt > 0 else { return Self.err(422, "invalid_link", "Enter an amount above zero.") }
            let id = nextID("pl")
            links[id] = Link(id: id, amount: amt, note: obj["description"] as? String, maxUses: (obj["max_uses"] as? NSNumber)?.intValue, expires: obj["expires_at"] as? String, canceled: false, created: now())
            linkOrder.append(id)
            return (201, (try? JSONSerialization.data(withJSONObject: ["id": id, "path": "/pay/link/demo/\(id)"])) ?? Data())
        case ("GET", 1, "payment_links"):
            return Self.ok(["data": linkOrder.reversed().compactMap { links[$0] }.map(json)])
        case ("POST", 3, "payment_links") where s[2] == "cancel":
            guard var l = links[s[1]], !l.canceled else { return Self.err(404, "not_found", "No active payment link.") }
            l.canceled = true; links[l.id] = l
            return Self.ok(["id": l.id, "status": "canceled"])
        case ("POST", 3, "payment_links") where s[2] == "send":
            return Self.err(409, "checkout_url_not_configured", "The built-in demo has no mail. Connect a test server to email a link.")
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
