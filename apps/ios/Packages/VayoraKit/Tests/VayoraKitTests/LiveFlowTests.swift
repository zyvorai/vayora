import XCTest
@testable import VayoraKit

/// The whole merchant -> payer -> merchant loop against a real running Vayora. Skipped unless
/// `VAYORA_E2E_URL` and `VAYORA_E2E_KEY` are set (a test tenant with a `sim` provider account, and the API
/// started with `ZYVOR_SIM_PAYER=1`). No real money is involved.
final class LiveFlowTests: XCTestCase {
    func liveClient() throws -> VayoraClient {
        let env = ProcessInfo.processInfo.environment
        guard let url = env["VAYORA_E2E_URL"], let key = env["VAYORA_E2E_KEY"], let u = URL(string: url) else {
            throw XCTSkip("set VAYORA_E2E_URL and VAYORA_E2E_KEY to run against a live Vayora")
        }
        return VayoraClient(baseURL: u, apiKey: key)
    }

    func testPayerPaysMerchantsQRAndEveryoneAgrees() async throws {
        let merchant = try liveClient()
        let payer = VayoraClient(baseURL: merchant.baseURL)          // the payer app has no API key

        let intent = try await merchant.createIntent(amount: Money(minor: 49900), note: "e2e")
        let attempt = try await merchant.createUPIAttempt(intentId: intent.id)
        let uri = try XCTUnwrap(attempt.upiURI, "no UPI payload issued")

        // What the payer app does with a scanned code.
        let parsed = try UPIPayload.parse(uri)
        XCTAssertEqual(parsed.amount, Money(minor: 49900))

        let still = try await merchant.refreshAttempt(id: attempt.id)
        XCTAssertEqual(still.status, "pending", "looking must not approve a payment the payer has not made")

        let paid = try await payer.simPay(upiURI: uri)
        XCTAssertEqual(paid.status, "captured")

        let done = try await merchant.refreshAttempt(id: attempt.id)
        XCTAssertTrue(done.isPaid)
        let final = try await merchant.intent(id: intent.id)
        XCTAssertEqual(final.status, "succeeded")

        // A second scan cannot pay twice.
        do { _ = try await payer.simPay(upiURI: uri); XCTFail("paid twice") }
        catch let e as APIError { XCTAssertEqual(e.status, 409) }

        let refund = try await merchant.refund(intentId: intent.id, amount: nil, reason: "e2e")
        XCTAssertEqual(refund.status, "processed")
        XCTAssertEqual(refund.amountMinor, 49900)
    }

    func testADeclinedPaymentFailsTheAttempt() async throws {
        let merchant = try liveClient()
        let intent = try await merchant.createIntent(amount: Money(minor: 12500), note: nil)
        let attempt = try await merchant.createUPIAttempt(intentId: intent.id)
        _ = try await VayoraClient(baseURL: merchant.baseURL).simPay(upiURI: try XCTUnwrap(attempt.upiURI), approve: false)
        let after = try await merchant.refreshAttempt(id: attempt.id)
        XCTAssertEqual(after.status, "failed")
        XCTAssertEqual(after.failureReason, "declined_by_payer")
        XCTAssertTrue(after.isFinal && !after.isPaid)
    }
}
