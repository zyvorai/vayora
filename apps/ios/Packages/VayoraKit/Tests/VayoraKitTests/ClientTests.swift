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
