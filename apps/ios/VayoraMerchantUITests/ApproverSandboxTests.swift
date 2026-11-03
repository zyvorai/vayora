import XCTest

/// A successful approval on a real seeded sandbox, by a second person: the payout and the refund were requested with the admin key,
/// so they are approved here with a different key (scopes: read, refunds:approve, payouts:approve, disputes:write) minted by
/// `APPROVER_KEY_OUT=<file> scripts/seed-sandbox-ios.sh`. Needs TEST_RUNNER_VAYORA_SANDBOX_URL and _APPROVER_KEY. Each run uses up the
/// pending payout, so re-seed before running it again.
final class ApproverSandboxTests: XCTestCase {
    var app: XCUIApplication!

    override func setUpWithError() throws {
        let env = ProcessInfo.processInfo.environment
        let url = try XCTUnwrap(env["VAYORA_SANDBOX_URL"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        let key = try XCTUnwrap(env["VAYORA_SANDBOX_APPROVER_KEY"].flatMap { $0.isEmpty ? nil : $0 }, "no approver key")
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-demoMode", "NO", "-cashierLocked", "NO", "-uitestServer", url, "-uitestKey", key]
        app.launch()
        app.tabBars.buttons["Owed"].tap()
    }

    func shot(_ name: String) { let a = XCTAttachment(screenshot: app.screenshot()); a.name = name; a.lifetime = .keepAlways; add(a) }

    func testASecondPersonCanApproveAPayoutAndARefund() {
        let payout = app.buttons["Approve payout of ₹1,200.00"].firstMatch
        XCTAssertTrue(payout.waitForExistence(timeout: 12), "seed the sandbox first")
        payout.tap()
        XCTAssertTrue(app.staticTexts["Payout approved."].waitForExistence(timeout: 10))
        let refund = app.buttons["Approve refund of ₹25.00"].firstMatch
        XCTAssertTrue(refund.waitForExistence(timeout: 10))
        refund.tap()
        XCTAssertTrue(app.staticTexts["Refund approved."].waitForExistence(timeout: 10), "the server's answer to a refund approval")
        shot("approver-after")
    }
}
