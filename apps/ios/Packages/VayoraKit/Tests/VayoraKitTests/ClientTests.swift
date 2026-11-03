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
