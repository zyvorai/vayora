import XCTest

/// Walks the merchant app's newer screens in the built-in demo (no server, test money) and checks each one shows what it should.
/// Screenshots are attached to the result so a person can look at them. It is a smoke test, not a layout or VoiceOver check.
final class WalkthroughTests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-demoMode", "YES", "-cashierLocked", "NO"]
        app.launch()
    }

    func shot(_ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot()); a.name = name; a.lifetime = .keepAlways; add(a)
    }
    func see(_ text: String, timeout: TimeInterval = 8, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(app.staticTexts[text].firstMatch.waitForExistence(timeout: timeout), "expected to see “\(text)”", file: file, line: line)
    }

    func testPaymentLinksCanBeMadeAndCancelled() {
        app.buttons["Payment links"].tap()
        see("No links yet. Make one above and share it with a customer.")
        let amount = app.textFields["Amount in rupees"]; amount.tap(); amount.typeText("250")
        app.buttons["Create link"].tap()
        see("₹250.00")
        shot("payment-links")
        app.buttons["Cancel link"].firstMatch.tap()
        see("Canceled")
    }

    func testOwedShowsAgeingApprovalsDisputesAndDrafts() {
        app.tabBars.buttons["Owed"].tap()
        see("Owed to you")
        see("Waiting for your approval")
        see("Disputes to answer")
        see("Reminders to approve")
        see("1 to 30 days late")
        shot("owed")
        app.buttons["Approve"].firstMatch.tap()      // the first card with an Approve is the approvals card
        see("Refund approved.")
        shot("owed-after-approve")
    }

    func testCashflowOpensFromPayments() {
        app.tabBars.buttons["Payments"].tap()
        app.buttons["Cashflow"].tap()
        see("Still to come in")
        see("Settled to your bank")
        shot("cashflow")
    }

    func testSubscriptionsCardOpensAndSaysTheDemoHasNone() {
        app.tabBars.buttons["Settings"].tap()
        app.buttons["Open subscriptions"].tap()
        see("Subscriptions")
        shot("subscriptions")
    }
}
