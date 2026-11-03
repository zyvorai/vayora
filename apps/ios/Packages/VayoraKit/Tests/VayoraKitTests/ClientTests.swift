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
