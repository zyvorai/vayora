import XCTest

/// The same screens against a real local test server instead of the built-in demo.
/// Skipped unless VAYORA_SANDBOX_URL and VAYORA_SANDBOX_KEY are set. Pass them to the runner as
/// TEST_RUNNER_VAYORA_SANDBOX_URL and TEST_RUNNER_VAYORA_SANDBOX_KEY (the sandbox script in scripts/ makes both).
/// The checks expect INV-1001 overdue; they pass on a seeded sandbox too. The key is never printed.
final class SandboxTests: XCTestCase {
    var app: XCUIApplication!

    override func setUpWithError() throws {
        let env = ProcessInfo.processInfo.environment
        let url = try XCTUnwrap(env["VAYORA_SANDBOX_URL"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        let key = try XCTUnwrap(env["VAYORA_SANDBOX_KEY"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        continueAfterFailure = false
        app = XCUIApplication()
        // a debug build reads these two (a test key only) instead of anyone typing into Settings
        app.launchArguments = ["-demoMode", "NO", "-cashierLocked", "NO", "-uitestServer", url, "-uitestKey", key]
        app.launch()
    }


    func shot(_ name: String) { let a = XCTAttachment(screenshot: app.screenshot()); a.name = name; a.lifetime = .keepAlways; add(a) }
    func see(_ text: String, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(app.staticTexts[text].firstMatch.waitForExistence(timeout: timeout), "expected to see “\(text)”", file: file, line: line)
    }

    func testOwedShowsTheSandboxInvoicesAndAnEmptyInbox() {
        app.tabBars.buttons["Owed"].tap()
        see("Owed to you")
        see("INV-1001")
        see("1 to 30 days late")
        shot("sandbox-owed")
    }

    func testPaymentLinkRoundTripAgainstTheRealServer() {
        app.tabBars.buttons["Collect"].tap()
        app.buttons["Payment links"].tap()
        let amount = app.textFields["Amount in rupees"]; amount.tap(); amount.typeText("321")
        app.buttons["Create link"].tap()
        see("₹321.00")
        shot("sandbox-links")
        app.buttons["Cancel link"].firstMatch.tap()
        see("Canceled")
    }

    func testCashflowAndTheTestClockOnTheRealServer() {
        app.tabBars.buttons["Payments"].tap()
        app.buttons["Cashflow"].tap()
        see("Still to come in")
        see("GST")
        shot("sandbox-cashflow")
        app.buttons["Close"].tap()
        app.tabBars.buttons["Settings"].tap()
        app.buttons["Open subscriptions"].tap()
        see("Test clock")
        app.buttons["+7 days"].tap()
        see("Moved forward 7 days.")
        shot("sandbox-clock")
        app.buttons["Reset"].tap()
        see("Clock back on the real date.")
    }

    /// INV-1001 has a customer with an address (it is queued for sending); INV-1002 has no customer, and the server's refusal is shown.
    /// The sandbox mailer only logs, so nothing is delivered; this checks the request and the messages, not a received email.
    func testEmailingAnInvoiceQueuesItAndARefusalIsShown() {
        app.tabBars.buttons["Owed"].tap()
        see("INV-1002")
        app.buttons["Actions for INV-1002"].tap()
        app.buttons["Email invoice"].tap()
        let refusal = app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Error:'")).firstMatch
        XCTAssertTrue(refusal.waitForExistence(timeout: 10), "an invoice with no customer cannot be emailed")
        XCTAssertTrue(refusal.isHittable, "the message must be on screen even though the list is scrolled")
        shot("sandbox-email-refused")
        app.buttons["Actions for INV-1001"].tap()
        app.buttons["Email invoice"].tap()
        // a second try on the same day is refused by the server (once per invoice per day), so a re-run shows that instead
        let ok = app.staticTexts["Invoice INV-1001 emailed."]
        let again = app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Error:'")).firstMatch
        XCTAssertTrue(ok.waitForExistence(timeout: 10) || again.waitForExistence(timeout: 2))
        shot("sandbox-email-sent")
    }

    /// A link made for a customer with an address can be emailed from the phone; the server answers (sent, or the sandbox's missing checkout
    /// address), and either way the answer is on screen. A link with no customer has no Email button.
    func testALinkForACustomerCanBeEmailed() {
        app.tabBars.buttons["Collect"].tap()
        app.buttons["Payment links"].tap()
        let amount = app.textFields["Amount in rupees"]; amount.tap(); amount.typeText("77")
        let picker = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'For'")).firstMatch
        XCTAssertTrue(picker.waitForExistence(timeout: 10), "the customer picker appears when the server has customers")
        picker.tap()
        app.buttons["Asha Rao"].firstMatch.tap()
        app.buttons["Create link"].tap()
        let email = app.buttons["Email"].firstMatch
        XCTAssertTrue(email.waitForExistence(timeout: 10), "a link for a customer with an address can be emailed")
        shot("sandbox-link-for-customer")
        email.tap()
        let sent = app.staticTexts["Link emailed."]
        let refused = app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Error:'")).firstMatch
        XCTAssertTrue(sent.waitForExistence(timeout: 10) || refused.waitForExistence(timeout: 3), "the server's answer is shown")
        if refused.exists { XCTAssertTrue(refused.label.contains("Ask whoever runs your Zyvor server"), "an operator's instruction is turned into plain words: \(refused.label)") }
        shot("sandbox-link-email-result")
        app.buttons["Cancel link"].firstMatch.tap()
    }
}
