import XCTest

/// The same screens against a real local test server instead of the built-in demo.
/// Skipped unless VAYORA_SANDBOX_URL and VAYORA_SANDBOX_KEY are set. Pass them to the runner as
/// TEST_RUNNER_VAYORA_SANDBOX_URL and TEST_RUNNER_VAYORA_SANDBOX_KEY (the sandbox script in scripts/ makes both).
/// The server should be fresh (the checks expect INV-1001 overdue and no subscriptions). The key is never printed.
final class SandboxTests: XCTestCase {
    var app: XCUIApplication!

    override func setUpWithError() throws {
        let env = ProcessInfo.processInfo.environment
        let url = try XCTUnwrap(env["VAYORA_SANDBOX_URL"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        let key = try XCTUnwrap(env["VAYORA_SANDBOX_KEY"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-demoMode", "NO", "-cashierLocked", "NO"]
        app.launch()
        app.tabBars.buttons["Settings"].tap()
        let server = app.textFields["https://api.example.com"]
        XCTAssertTrue(server.waitForExistence(timeout: 8))
        replace(server, with: url)
        replace(app.secureTextFields["zk_test_…"], with: key)
    }

    /// The server address and key survive between launches (the key lives in the Keychain), so clear a field before typing into it.
    func replace(_ field: XCUIElement, with text: String) {
        field.tap()
        let current = (field.value as? String) ?? ""
        if !current.isEmpty, current != field.placeholderValue {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count + 5))
        }
        field.typeText(text)
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
        XCTAssertFalse(app.staticTexts["Waiting for your approval"].exists, "a fresh sandbox has nothing to approve")
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
}
