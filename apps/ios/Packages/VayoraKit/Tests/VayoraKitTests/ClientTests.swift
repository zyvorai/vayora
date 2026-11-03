import XCTest
@testable import VayoraKit

final class StubProtocol: URLProtocol {
    nonisolated(unsafe) static var handler: ((URLRequest) -> (Int, String))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (code, body) = Self.handler!(request)
        let resp = HTTPURLResponse(url: request.url!, statusCode: code, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: resp, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

final class ClientTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testCreateIntentSendsMinorUnitsAndIdempotencyKey() async throws {
        nonisolated(unsafe) var seen: URLRequest?
        StubProtocol.handler = { r in
            seen = r
            return (200, #"{"id":"pi_1","amount_minor":49900,"currency":"INR","status":"requires_payment","client_secret":"cs"}"#)
        }
        let i = try await client().createIntent(amount: Money(minor: 49900), note: "Tea", idempotencyKey: "idem-1")
        XCTAssertEqual(i.money, Money(minor: 49900))
        XCTAssertEqual(seen?.value(forHTTPHeaderField: "Idempotency-Key"), "idem-1")
        XCTAssertEqual(seen?.value(forHTTPHeaderField: "Authorization"), "Bearer k")
        XCTAssertEqual(seen?.url?.path, "/v1/payment_intents")
    }

    func testServerErrorsKeepTheirCode() async {
        StubProtocol.handler = { _ in (404, #"{"error":{"code":"not_found","message":"route not found"}}"#) }
        do { _ = try await client().simPay(upiURI: "upi://pay?pa=a@b"); XCTFail("should throw") }
        catch let e as APIError { XCTAssertEqual(e.status, 404); XCTAssertEqual(e.code, "not_found") }
        catch { XCTFail("\(error)") }
    }
}

final class PaymentLinkTests: XCTestCase {
    /// URLSession hands a protocol the body as a stream.
    static func bodyData(_ r: URLRequest?) -> Data {
        if let b = r?.httpBody { return b }
        guard let st = r?.httpBodyStream else { return Data() }
        st.open(); defer { st.close() }
        var out = Data(); var buf = [UInt8](repeating: 0, count: 1024)
        while st.hasBytesAvailable { let n = st.read(&buf, maxLength: buf.count); if n <= 0 { break }; out.append(buf, count: n) }
        return out
    }
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testListDecodesAndCreateSendsMinorUnitsWithAKey() async throws {
        StubProtocol.handler = { _ in (200, #"{"data":[{"id":"pl_1","path":"/pay/link/t1/tok","description":"Rent","amount_minor":150000,"currency":"INR","invoice_id":null,"max_uses":1,"uses":0,"expires_at":null,"status":"active","created_at":"2026-11-03T10:00:00Z","can_email":true}]}"#) }
        let l = try await client().paymentLinks()
        XCTAssertEqual(l.first?.amountMinor, 150000); XCTAssertEqual(l.first?.canEmail, true); XCTAssertTrue(l.first?.isActive ?? false)

        nonisolated(unsafe) var seen: URLRequest?
        StubProtocol.handler = { r in seen = r; return (201, #"{"id":"pl_2","path":"/pay/link/t1/tok2"}"#) }
        let made = try await client().createPaymentLink(amount: Money(minor: 25000), description: "Tea", maxUses: 1, idempotencyKey: "k-1")
        XCTAssertEqual(made.id, "pl_2")
        XCTAssertEqual(seen?.value(forHTTPHeaderField: "Idempotency-Key"), "k-1")
        XCTAssertEqual(seen?.url?.path, "/v1/payment_links")
        let body = try JSONSerialization.jsonObject(with: Self.bodyData(seen)) as? [String: Any]
        XCTAssertEqual((body?["amount_minor"] as? NSNumber)?.int64Value, 25000)
        XCTAssertEqual((body?["max_uses"] as? NSNumber)?.intValue, 1)
        XCTAssertNil(body?["customer_id"], "no customer unless one is chosen")
        _ = try await client().createPaymentLink(amount: Money(minor: 100), description: nil, customerID: "cus_1", idempotencyKey: "k-2")
        let withCustomer = try JSONSerialization.jsonObject(with: Self.bodyData(seen)) as? [String: Any]
        XCTAssertEqual(withCustomer?["customer_id"] as? String, "cus_1")
    }

    func testCancelAndEmailHitTheirPaths() async throws {
        nonisolated(unsafe) var paths: [String] = []
        StubProtocol.handler = { r in paths.append(r.url!.path); return (200, #"{"id":"pl_1","status":"canceled"}"#) }
        try await client().cancelPaymentLink(id: "pl_1")
        try await client().emailPaymentLink(id: "pl_1")
        XCTAssertEqual(paths, ["/v1/payment_links/pl_1/cancel", "/v1/payment_links/pl_1/send"])
    }

    func testShareURLNeedsAnHttpsAddressAndALinkPath() {
        let l = PaymentLinkInfo(id: "pl_1", path: "/pay/link/t1/tok")
        XCTAssertEqual(l.shareURL(checkoutBase: "https://pay.example.com")?.absoluteString, "https://pay.example.com/pay/link/t1/tok")
        XCTAssertEqual(l.shareURL(checkoutBase: " https://pay.example.com/ ")?.absoluteString, "https://pay.example.com/pay/link/t1/tok")
        XCTAssertNil(l.shareURL(checkoutBase: "http://pay.example.com"))
        XCTAssertNil(l.shareURL(checkoutBase: ""))
        XCTAssertNil(l.shareURL(checkoutBase: "https://user:pw@pay.example.com"))
        XCTAssertNil(PaymentLinkInfo(id: "x", path: "/pay/link/../admin").shareURL(checkoutBase: "https://pay.example.com"))
        XCTAssertNil(PaymentLinkInfo(id: "x", path: "/other").shareURL(checkoutBase: "https://pay.example.com"))
    }

    func testDemoBankCreatesListsAndCancelsLinks() async throws {
        let c = VayoraClient.demo(bank: DemoBank())
        let made = try await c.createPaymentLink(amount: Money(minor: 9900), description: "Class fee")
        var l = try await c.paymentLinks()
        XCTAssertEqual(l.map(\.id), [made.id]); XCTAssertTrue(l[0].isActive)
        try await c.cancelPaymentLink(id: made.id)
        l = try await c.paymentLinks()
        XCTAssertFalse(l[0].isActive)
    }
}

final class CollectionsTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testAgeingDecodesAndOrdersMostOverdueFirstInRupeesOnly() async throws {
        StubProtocol.handler = { _ in (200, #"{"as_of":"2026-11-03","total_outstanding_inr_minor":null,"data":[{"currency":"INR","bucket":"current","outstanding_minor":500,"count":1,"outstanding_inr_minor":500,"invoices":[{"id":"i1","number":"A","customer_id":null,"due_date":"2026-11-09","outstanding_minor":500,"days_past_due":0}]},{"currency":"USD","bucket":"over_90","outstanding_minor":900,"count":1,"invoices":[]},{"currency":"INR","bucket":"over_90","outstanding_minor":7000,"count":1,"invoices":[{"id":"i2","number":"B","due_date":"2026-06-01","outstanding_minor":7000,"days_past_due":155}]}]}"#) }
        let a = try await client().ageing()
        XCTAssertEqual(a.inrBuckets.map(\.bucket), ["over_90", "current"])
        XCTAssertEqual(a.inrTotalMinor, 7500)
        XCTAssertEqual(a.inrBuckets.first?.title, "Over 90 days late")
        XCTAssertEqual(a.inrBuckets.first?.invoices.first?.daysPastDue, 155)
    }

    func testDraftApprovalSendsEditedWordingOnlyWhenGiven() async throws {
        nonisolated(unsafe) var bodies: [Data] = []; nonisolated(unsafe) var paths: [String] = []
        StubProtocol.handler = { r in paths.append(r.url!.path); bodies.append(PaymentLinkTests.bodyData(r)); return (200, #"{"id":"rd","status":"approved"}"#) }
        try await client().approveReminderDraft(id: "rd")
        try await client().approveReminderDraft(id: "rd", message: "Please pay")
        try await client().rejectReminderDraft(id: "rd")
        XCTAssertEqual(paths, ["/v1/collections/drafts/rd/approve", "/v1/collections/drafts/rd/approve", "/v1/collections/drafts/rd/reject"])
        XCTAssertEqual(String(data: bodies[0], encoding: .utf8), "{}")   // never an empty body: the server answers that with a 400
        XCTAssertEqual((try JSONSerialization.jsonObject(with: bodies[1]) as? [String: Any])?["message"] as? String, "Please pay")
    }

    func testPromiseAndInvoiceEmail() async throws {
        nonisolated(unsafe) var seen: [URLRequest] = []
        StubProtocol.handler = { r in seen.append(r); return (201, #"{"id":"pr_1"}"#) }
        try await client().promiseToPay(invoiceID: "in_1", amount: Money(minor: 5000), date: "2026-11-10", idempotencyKey: "k-9")
        try await client().emailInvoice(id: "in_1")
        XCTAssertEqual(seen[0].url?.path, "/v1/invoices/in_1/promises")
        XCTAssertEqual(seen[0].value(forHTTPHeaderField: "Idempotency-Key"), "k-9")
        let b = try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[0])) as? [String: Any]
        XCTAssertEqual((b?["promised_minor"] as? NSNumber)?.int64Value, 5000); XCTAssertEqual(b?["promised_date"] as? String, "2026-11-10")
        XCTAssertEqual(seen[1].url?.path, "/v1/invoices/in_1/send")
    }

    func testDemoBankDraftsLeaveTheInboxOnceDecided() async throws {
        let c = VayoraClient.demo(bank: DemoBank())
        let first = try await c.reminderDrafts()
        XCTAssertEqual(first.count, 2)
        try await c.approveReminderDraft(id: first[0].id)
        try await c.rejectReminderDraft(id: first[1].id)
        let left = try await c.reminderDrafts()
        XCTAssertTrue(left.isEmpty)
        let a = try await c.ageing()
        XCTAssertEqual(a.inrTotalMinor, 205000)
    }
}

final class ApprovalTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testPendingApprovalsMergeRefundsAndPayoutsAndAskForTheRightStatus() async throws {
        nonisolated(unsafe) var queries: [String] = []
        StubProtocol.handler = { r in
            queries.append(r.url?.absoluteString ?? "")
            if r.url!.path.hasSuffix("refunds") { return (200, #"{"data":[{"id":"re_1","amount_minor":500,"currency":"INR","status":"pending_approval","reason":"Wrong item","requested_by":"a"},{"id":"re_2","amount_minor":9,"currency":"INR","status":"failed","reason":null,"requested_by":"a"}],"has_more":false}"#) }
            return (200, #"{"data":[{"id":"po_1","amount_minor":90000,"currency":"INR","status":"pending_approval","purpose":"Rent","requested_by":"b"}]}"#)
        }
        let a = try await client().pendingApprovals()
        XCTAssertEqual(a.map(\.id), ["refund:re_1", "payout:po_1"])
        XCTAssertEqual(a[0].detail, "Wrong item"); XCTAssertEqual(a[1].title, "Payout")
        XCTAssertTrue(queries.allSatisfy { $0.hasSuffix("status=pending_approval") })
    }

    func testDecisionsGoToTheRightPathAndSeparationOfDutiesIsShown() async throws {
        nonisolated(unsafe) var paths: [String] = []
        StubProtocol.handler = { r in paths.append(r.url!.path); return (200, #"{"id":"x","status":"approved"}"#) }
        try await client().decide(PendingApproval(kind: .refund, rawID: "re_1", amountMinor: 1), approve: true)
        try await client().decide(PendingApproval(kind: .payout, rawID: "po_1", amountMinor: 1), approve: false)
        XCTAssertEqual(paths, ["/v1/refunds/re_1/approve", "/v1/payouts/po_1/reject"])
        StubProtocol.handler = { _ in (403, #"{"error":{"code":"separation_of_duties","message":"the person who requested a payout cannot approve it"}}"#) }
        do { try await client().decide(PendingApproval(kind: .payout, rawID: "po_1", amountMinor: 1), approve: true); XCTFail("should throw") }
        catch let e as APIError { XCTAssertEqual(e.code, "separation_of_duties"); XCTAssertEqual(e.status, 403) }
    }

    func testDemoBankApprovalLeavesTheInbox() async throws {
        let c = VayoraClient.demo(bank: DemoBank())
        let first = try await c.pendingApprovals()
        XCTAssertEqual(first.count, 2)
        for a in first { try await c.decide(a, approve: true) }
        let left = try await c.pendingApprovals()
        XCTAssertTrue(left.isEmpty)
    }
}

final class DisputeTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testOnlyOpenDisputesAreListedAndDaysLeftCountsInIndianDays() async throws {
        StubProtocol.handler = { _ in (200, #"{"data":[{"id":"d1","amount_minor":45000,"reason":"duplicate","status":"open","respond_by":"2026-11-10","evidence_count":0},{"id":"d2","amount_minor":1,"reason":"x","status":"won","respond_by":null,"evidence_count":2},{"id":"d3","amount_minor":9,"reason":"y","status":"under_review","respond_by":null,"evidence_count":1}]}"#) }
        let d = try await client().openDisputes()
        XCTAssertEqual(d.map(\.id), ["d1", "d3"])
        XCTAssertEqual(d[0].daysLeft(today: "2026-11-03"), 7)
        XCTAssertEqual(d[0].daysLeft(today: "2026-11-12"), -2)
        XCTAssertNil(d[1].daysLeft(today: "2026-11-03"))
    }

    func testEvidenceAndOutcomeBodies() async throws {
        nonisolated(unsafe) var seen: [URLRequest] = []
        StubProtocol.handler = { r in seen.append(r); return (200, #"{"id":"d1","status":"under_review"}"#) }
        try await client().addDisputeEvidence(id: "d1", note: "Delivered, signed receipt", url: "https://files.example.com/r.pdf")
        try await client().resolveDispute(id: "d1", won: false)
        let e = try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[0])) as? [String: Any]
        XCTAssertEqual(e?["note"] as? String, "Delivered, signed receipt"); XCTAssertEqual(e?["url"] as? String, "https://files.example.com/r.pdf")
        XCTAssertEqual(seen[0].url?.path, "/v1/disputes/d1/evidence")
        let r = try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[1])) as? [String: Any]
        XCTAssertEqual(r?["outcome"] as? String, "lost"); XCTAssertEqual(seen[1].url?.path, "/v1/disputes/d1/resolve")
    }

    func testDemoBankEvidenceThenResolveClosesIt() async throws {
        let c = VayoraClient.demo(bank: DemoBank())
        try await c.addDisputeEvidence(id: "dp_demo1", note: "Receipt")
        var d = try await c.openDisputes()
        XCTAssertEqual(d.first?.evidenceCount, 1)
        do { try await c.addDisputeEvidence(id: "dp_demo1", note: "  "); XCTFail("blank note") } catch let e as APIError { XCTAssertEqual(e.code, "note_required") }
        try await c.resolveDispute(id: "dp_demo1", won: true)
        d = try await c.openDisputes()
        XCTAssertTrue(d.isEmpty)
    }
}

final class CashflowTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testCashflowDecodesAndTotalsEverythingOwed() async throws {
        nonisolated(unsafe) var url = ""
        StubProtocol.handler = { r in url = r.url!.absoluteString; return (200, #"{"as_of":"2026-11-03","currency":"INR","weeks":[{"starts":"2026-11-03","due_minor":100},{"starts":"2026-11-10","due_minor":200}],"overdue_minor":50,"due_later_minor":7,"no_due_date_minor":3,"promised_minor":40,"payouts_waiting_minor":900,"daily_receipts_run_rate_minor":1000,"note":"x"}"#) }
        let c = try await client().cashflow(weeks: 99)
        XCTAssertEqual(c.totalOwedMinor, 360); XCTAssertEqual(c.payoutsWaitingMinor, 900); XCTAssertEqual(c.weeks.count, 2)
        XCTAssertTrue(url.hasSuffix("/v1/insights/cashflow?weeks=12"))   // clamped, and the query stays a query
    }

    func testSettlementDeductionsAndDemo() async throws {
        let s = SettlementInfo(id: "s", provider: "x", grossMinor: 1000, netMinor: 940)
        XCTAssertEqual(s.deductionsMinor, 60)
        let c = VayoraClient.demo(bank: DemoBank())
        let list = try await c.settlements()
        XCTAssertEqual(list.first?.deductionsMinor, 36800)
        let f = try await c.cashflow()
        XCTAssertEqual(f.totalOwedMinor, 205000)
        let owed = try await c.ageing()
        XCTAssertEqual(f.totalOwedMinor, owed.inrTotalMinor)   // the two demo screens agree
    }
}

final class SubscriptionTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testPlansAndSubscriptionsDecodeAndStatesDecideTheActions() async throws {
        StubProtocol.handler = { r in
            if r.url!.path.hasSuffix("plans") { return (200, #"{"data":[{"id":"pl","name":"Gold","currency":"INR","amount_minor":49900,"interval_unit":"month","interval_count":1,"usage_unit_minor":null,"active":true,"trial_days":0}]}"#) }
            return (200, #"{"data":[{"id":"s1","customer_id":"c","plan_id":"pl","quantity":2,"status":"active","next_bill_date":"2026-12-01","unbilled_usage":0,"unit_amount_minor":null,"trial_ends_on":null,"billed":true},{"id":"s2","customer_id":"c","plan_id":"pl","quantity":1,"status":"paused","next_bill_date":"2026-12-01","trial_ends_on":null},{"id":"s3","customer_id":"c","plan_id":"pl","quantity":1,"status":"canceled","next_bill_date":"2026-12-01"}]}"#)
        }
        let p = try await client().plans()
        XCTAssertEqual(p[0].priceLine, "\(Money(minor: 49900).display) every month")
        XCTAssertEqual(PlanInfo(id: "x", name: "Q", amountMinor: 100, intervalCount: 3).priceLine, "\(Money(minor: 100).display) every 3 months")
        let s = try await client().subscriptions()
        XCTAssertEqual([s[0].canPause, s[0].canResume, s[0].canCancel], [true, false, true])
        XCTAssertEqual([s[1].canPause, s[1].canResume, s[1].canCancel], [false, true, true])
        XCTAssertEqual([s[2].canPause, s[2].canResume, s[2].canCancel], [false, false, false])
    }

    func testActionsAndClockRequests() async throws {
        nonisolated(unsafe) var seen: [URLRequest] = []
        StubProtocol.handler = { r in seen.append(r); return (200, #"{"date":"2026-11-10","frozen":true,"real_date":"2026-11-03"}"#) }
        try await client().pauseSubscription(id: "s1"); try await client().resumeSubscription(id: "s1"); try await client().cancelSubscription(id: "s1")
        let c = try await client().advanceTestClock(days: 7)
        _ = try await client().resetTestClock()
        XCTAssertEqual(seen.map { "\($0.httpMethod ?? "") \($0.url!.path)" }, ["POST /v1/subscriptions/s1/pause", "POST /v1/subscriptions/s1/resume", "POST /v1/subscriptions/s1/cancel", "POST /v1/test_clock", "DELETE /v1/test_clock"])
        XCTAssertEqual(c.date, "2026-11-10"); XCTAssertTrue(c.frozen)
        let b = try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[3])) as? [String: Any]
        XCTAssertEqual((b?["advance_days"] as? NSNumber)?.intValue, 7)
    }

    func testTheClockIsNotInTheBuiltInDemo() async {
        do { _ = try await VayoraClient.demo(bank: DemoBank()).testClock(); XCTFail("demo has no clock") }
        catch let e as APIError { XCTAssertEqual(e.code, "not_in_demo") } catch { XCTFail("\(error)") }
    }
}

final class AttentionTests: XCTestCase {
    func testCountsApprovalsDraftsAndOnlyDisputesDueSoon() {
        let a = Attention(
            approvals: [PendingApproval(kind: .refund, rawID: "r", amountMinor: 1), PendingApproval(kind: .payout, rawID: "p", amountMinor: 1)],
            drafts: [ReminderDraft(id: "d", message: "m")],
            disputes: [DisputeInfo(id: "1", amountMinor: 1, reason: "x", respondBy: "2026-11-05"),     // 2 days: counts
                       DisputeInfo(id: "2", amountMinor: 1, reason: "x", respondBy: "2026-11-20"),     // far off: no
                       DisputeInfo(id: "3", amountMinor: 1, reason: "x", respondBy: "2026-11-01"),     // passed: counts
                       DisputeInfo(id: "4", amountMinor: 1, reason: "x", status: "won", respondBy: "2026-11-04"),   // closed: no
                       DisputeInfo(id: "5", amountMinor: 1, reason: "x")],                             // no deadline: no
            today: "2026-11-03")
        XCTAssertEqual(a.total, 5)
        XCTAssertEqual(a.summary, "2 to approve, 1 reminder, 2 disputes due soon")
        XCTAssertNil(Attention().summary)
    }

    func testPayoutsToPayAndOpenCasesCountToo() {
        let a = Attention(approvals: [], drafts: [], disputes: [], today: "2026-11-03",
                          payouts: [ApprovedPayout(id: "p", amountMinor: 1)],
                          cases: [ReconCase(id: "c1", kind: "x"), ReconCase(id: "c2", kind: "y", status: "assigned"), ReconCase(id: "c3", kind: "z", status: "resolved")])
        XCTAssertEqual(a.total, 3)       // one payout, two unresolved cases; the resolved one does not count
        XCTAssertEqual(a.summary, "1 payout to pay, 2 cases to check")
    }

    func testTheCountWorksWhenAListIsRefused() async {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        let c = VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
        StubProtocol.handler = { r in
            if r.url!.path.contains("drafts") { return (200, #"{"data":[{"id":"d","invoice_number":"A","channel":"email","message":"m"}]}"#) }
            return (403, #"{"error":{"code":"forbidden","message":"no"}}"#)
        }
        let a = await c.attention(today: "2026-11-03")
        XCTAssertEqual(a, Attention(approvals: 0, reminderDrafts: 1, disputesDueSoon: 0))
    }
}

final class PayoutActionTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testBankRefRuleMatchesTheServer() {
        XCTAssertFalse(BankRef.isValid("12345")); XCTAssertTrue(BankRef.isValid("123456")); XCTAssertTrue(BankRef.isValid("  UTR0001234  "))
        XCTAssertFalse(BankRef.isValid(String(repeating: "9", count: 41))); XCTAssertFalse(BankRef.isValid("   abc   "))
    }

    func testMarkPaidSendsTheTrimmedReferenceAndListsAskForTheirStatus() async throws {
        nonisolated(unsafe) var seen: [URLRequest] = []
        StubProtocol.handler = { r in
            seen.append(r)
            if r.httpMethod == "GET" && r.url!.path.hasSuffix("payouts") { return (200, #"{"data":[{"id":"po_1","amount_minor":9900,"currency":"INR","status":"approved","purpose":"Rent"},{"id":"po_2","amount_minor":1,"currency":"INR","status":"paid"}]}"#) }
            if r.httpMethod == "GET" { return (200, #"{"data":[{"id":"re_1","amount_minor":500,"currency":"INR","status":"failed","failure_reason":"bank said no"}],"has_more":false}"#) }
            return (200, #"{"id":"x","status":"ok"}"#)
        }
        let p = try await client().approvedPayouts()
        XCTAssertEqual(p.map(\.id), ["po_1"])
        let f = try await client().failedRefunds()
        XCTAssertEqual(f.first?.failureReason, "bank said no")
        try await client().markPayoutPaid(id: "po_1", bankRef: "  UTR0001234 ")
        try await client().retryRefund(id: "re_1")
        XCTAssertTrue(seen[0].url!.absoluteString.hasSuffix("/v1/payouts?status=approved"))
        XCTAssertTrue(seen[1].url!.absoluteString.hasSuffix("/v1/refunds?status=failed"))
        XCTAssertEqual(seen[2].url?.path, "/v1/payouts/po_1/mark_paid")
        let b = try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[2])) as? [String: Any]
        XCTAssertEqual(b?["bank_ref"] as? String, "UTR0001234")
        XCTAssertEqual(seen[3].url?.path, "/v1/refunds/re_1/retry")
    }

    func testDemoBankPaysOnceAndRetriesAFailedRefund() async throws {
        let c = VayoraClient.demo(bank: DemoBank())
        let p = try await c.approvedPayouts(); XCTAssertEqual(p.count, 1)
        do { try await c.markPayoutPaid(id: p[0].id, bankRef: "abc"); XCTFail("short reference") } catch let e as APIError { XCTAssertEqual(e.code, "invalid_bank_ref") }
        try await c.markPayoutPaid(id: p[0].id, bankRef: "UTR123456789")
        let none = try await c.approvedPayouts(); XCTAssertTrue(none.isEmpty)
        let f = try await c.failedRefunds(); XCTAssertEqual(f.count, 1)
        try await c.retryRefund(id: f[0].id)
        let left = try await c.failedRefunds(); XCTAssertTrue(left.isEmpty)
        // the pending-approval lists are untouched by the new filters
        let pending = try await c.pendingApprovals(); XCTAssertEqual(pending.count, 2)
    }
}

final class GSTTests: XCTestCase {
    func testPeriodsRollOverYearsAndNameThemselves() {
        XCTAssertEqual(GSTPeriod(year: 2026, month: 1).shifted(by: -1), GSTPeriod(year: 2025, month: 12))
        XCTAssertEqual(GSTPeriod(year: 2026, month: 12).shifted(by: 1), GSTPeriod(year: 2027, month: 1))
        XCTAssertEqual(GSTPeriod(year: 2026, month: 3).shifted(by: -14).id, "2025-01")
        XCTAssertEqual(GSTPeriod(year: 2026, month: 10).id, "2026-10")
        XCTAssertTrue(GSTPeriod(year: 2026, month: 10).title.contains("2026"))
        // 23:30 UTC on 31 October is already 1 November in India
        let utc = ISO8601DateFormatter().date(from: "2026-10-31T23:30:00Z")!
        XCTAssertEqual(GSTPeriod.containing(utc), GSTPeriod(year: 2026, month: 11))
    }

    func testSummaryReadsTotalsAndCountsInvoicesAndCreditNotes() async throws {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        let c = VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
        nonisolated(unsafe) var url = ""
        StubProtocol.handler = { r in url = r.url!.absoluteString; return (200, #"{"note":"x","period":"2026-10","gstin":"29ABCDE1234F1Z5","legal_name":"A","b2b":[{"buyer_gstin":"27X","invoices":[{"number":"1"},{"number":"2"}]},{"buyer_gstin":"27Y","invoices":[{"number":"3"}]}],"b2cs":[],"cdnr":[{"note_id":"a"}],"cdnur":[{"note_id":"b"},{"note_id":"c"}],"hsn":[],"totals":{"taxable_minor":500000,"cgst_minor":45000,"sgst_minor":45000,"igst_minor":10000}}"#) }
        let s = try await c.gstSummary(period: GSTPeriod(year: 2026, month: 10))
        XCTAssertTrue(url.hasSuffix("/v1/gst/gstr1?period=2026-10"))
        XCTAssertEqual(s.taxableMinor, 500000); XCTAssertEqual(s.taxMinor, 100000); XCTAssertEqual(s.b2bInvoices, 3); XCTAssertEqual(s.creditNotes, 3)
        XCTAssertFalse(s.isEmpty)
        StubProtocol.handler = { _ in (200, #"{"period":"2026-09","gstin":null,"b2b":[],"b2cs":[],"cdnr":[],"cdnur":[],"hsn":[],"totals":{"taxable_minor":0,"cgst_minor":0,"sgst_minor":0,"igst_minor":0}}"#) }
        let empty = try await c.gstSummary(period: GSTPeriod(year: 2026, month: 9))
        XCTAssertTrue(empty.isEmpty)
        let demo = try await VayoraClient.demo(bank: DemoBank()).gstSummary(period: GSTPeriod(year: 2026, month: 10))
        XCTAssertEqual(demo.taxMinor, 180000); XCTAssertEqual(demo.b2bInvoices, 2); XCTAssertEqual(demo.creditNotes, 1)
    }
}

final class TimelineTests: XCTestCase {
    func testEventsAreShownOldestFirstInIndianTimeAndDecodeWithoutTheFreeFormDetail() async throws {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        let c = VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
        nonisolated(unsafe) var path = ""
        StubProtocol.handler = { r in path = r.url!.path; return (200, #"{"payment_id":"pi_1","status":"succeeded","standing":"Paid.","events":[{"at":"2026-11-03T12:50:00.500000Z","kind":"captured","title":"Paid","detail":{"x":1}},{"at":"2026-11-03T12:49:26.411235Z","kind":"intent","title":"Created","detail":{"external_ref":null}}]}"#) }
        let t = try await c.timeline(intentId: "pi_1")
        XCTAssertEqual(path, "/v1/payment_intents/pi_1/timeline")
        XCTAssertEqual(t.ordered.map(\.kind), ["intent", "captured"])
        XCTAssertEqual(t.ordered.first?.when, "3 Nov, 6:19 PM")          // 12:49 UTC is 18:19 in India
        XCTAssertEqual(TimelineEvent(at: "garbage", kind: "k", title: "t").when, "garbage")
        XCTAssertEqual(t.standing, "Paid.")
    }

    func testDemoTimelineGrowsAsThePaymentDoes() async throws {
        let c = VayoraClient.demo(bank: DemoBank(customerPaysAfter: 0))
        let i = try await c.createIntent(amount: Money(minor: 5000), note: nil)
        var t = try await c.timeline(intentId: i.id)
        XCTAssertEqual(t.events.count, 1)
        let a = try await c.createUPIAttempt(intentId: i.id)
        _ = try await c.refreshAttempt(id: a.id)
        t = try await c.timeline(intentId: i.id)
        XCTAssertEqual(t.ordered.map(\.kind), ["intent", "attempt", "captured"])
        XCTAssertEqual(t.standing, "Paid.")
    }
}

final class SettlementExplainTests: XCTestCase {
    func testExplanationDecodesAndSaysThingsPlainly() async throws {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        let c = VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
        nonisolated(unsafe) var path = ""
        StubProtocol.handler = { r in path = r.url!.path; return (200, #"{"settlement":{"id":"stl_1"},"payments":[{"matched":true},{"matched":false}],"refunds":[],"adjustments":[],"fees":{"observed_minor":3000},"arithmetic":{"balances":false,"gross_minor":1},"expected_minor":100000,"received_minor":97000,"difference_minor":-3000,"difference_breakdown":[{"reason":"provider fees and taxes (no fee schedule configured)","amount_minor":-3000}],"bank_match":{"posted_on":"2026-11-03"},"cases":[{"id":"c1","kind":"short_settlement","status":"open","explanation":"x","assignee":null}],"narrative":"Cashfree paid ₹970.00."}"#) }
        let e = try await c.explainSettlement(id: "stl_1")
        XCTAssertEqual(path, "/v1/settlements/stl_1/explain")
        XCTAssertEqual(e.verdict, "\(Money(minor: 3000).display) less than expected.")
        XCTAssertEqual([e.paymentsMatched, e.paymentsUnmatched], [1, 1]); XCTAssertFalse(e.balances); XCTAssertTrue(e.matchedBank)
        XCTAssertEqual(e.breakdown.first?.amountMinor, -3000); XCTAssertEqual(e.cases.first?.kind, "short_settlement")
        StubProtocol.handler = { _ in (200, #"{"payments":[],"cases":[],"bank_match":null,"arithmetic":{"balances":true},"expected_minor":5,"received_minor":5,"difference_minor":0,"difference_breakdown":[],"narrative":"ok"}"#) }
        let same = try await c.explainSettlement(id: "x")
        XCTAssertEqual(same.verdict, "Matches what was expected."); XCTAssertFalse(same.matchedBank)
        XCTAssertEqual(SettlementExplanation(narrative: "n", expectedMinor: 100, receivedMinor: 150).verdict, "\(Money(minor: 50).display) more than expected.")
        let demo = try await VayoraClient.demo(bank: DemoBank()).explainSettlement(id: "st_demo1")
        XCTAssertEqual(demo.differenceMinor, -30000); XCTAssertEqual(demo.breakdown.count, 2)
    }
}

final class ReconCaseTests: XCTestCase {
    func client() -> VayoraClient {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.protocolClasses = [StubProtocol.self]
        return VayoraClient(baseURL: URL(string: "https://api.test")!, apiKey: "k", session: URLSession(configuration: cfg))
    }

    func testOnlyUnresolvedCasesAreListedAndTitlesAreReadable() async throws {
        StubProtocol.handler = { _ in (200, #"{"data":[{"id":"c1","kind":"settlement_arithmetic","status":"open","subject_type":"settlement","subject_id":"s","expected_minor":1,"actual_minor":2,"diff_minor":-30000,"explanation":"x","assignee":null,"created_at":"2026-11-03T00:00:00Z"},{"id":"c2","kind":"unresolved_payment","status":"resolved","diff_minor":null,"explanation":null,"assignee":"asha"},{"id":"c3","kind":"duplicate_payment","status":"assigned","assignee":"ravi"}]}"#) }
        let cases = try await client().openReconCases()
        XCTAssertEqual(cases.map(\.id), ["c1", "c3"])
        XCTAssertEqual(cases[0].title, "Settlement Arithmetic"); XCTAssertEqual(cases[0].diffMinor, -30000); XCTAssertEqual(cases[1].assignee, "ravi")
    }

    func testAssignAndResolveSendTrimmedTextToTheirPaths() async throws {
        nonisolated(unsafe) var seen: [URLRequest] = []
        StubProtocol.handler = { r in seen.append(r); return (200, #"{"id":"c1","status":"assigned"}"#) }
        try await client().assignReconCase(id: "c1", to: "  Asha  ")
        try await client().resolveReconCase(id: "c1", resolution: " Bank confirmed the fee ")
        XCTAssertEqual(seen.map(\.url!.path), ["/v1/recon/cases/c1/assign", "/v1/recon/cases/c1/resolve"])
        XCTAssertEqual((try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[0])) as? [String: Any])?["assignee"] as? String, "Asha")
        XCTAssertEqual((try JSONSerialization.jsonObject(with: PaymentLinkTests.bodyData(seen[1])) as? [String: Any])?["resolution"] as? String, "Bank confirmed the fee")
        XCTAssertFalse(CaseNote.isUsable("  ok")); XCTAssertTrue(CaseNote.isUsable("fee ok"))
    }

    func testDemoBankAssignsThenResolves() async throws {
        let c = VayoraClient.demo(bank: DemoBank())
        var open = try await c.openReconCases(); XCTAssertEqual(open.count, 1); XCTAssertNil(open[0].assignee)
        try await c.assignReconCase(id: open[0].id, to: "Asha")
        open = try await c.openReconCases(); XCTAssertEqual(open[0].assignee, "Asha"); XCTAssertEqual(open[0].status, "assigned")
        try await c.resolveReconCase(id: open[0].id, resolution: "Fee confirmed")
        let none = try await c.openReconCases(); XCTAssertTrue(none.isEmpty)
    }
}
