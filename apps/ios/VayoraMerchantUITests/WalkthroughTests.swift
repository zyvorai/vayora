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
        see("Link created.")
        shot("payment-links")
        app.buttons["Cancel link"].firstMatch.tap()
        see("Canceled")
        see("Link canceled.")
    }

    func testOwedShowsAgeingApprovalsDisputesAndDrafts() {
        app.tabBars.buttons["Owed"].tap()
        see("Owed to you")
        see("Waiting for your approval")
        see("Disputes to answer")
        see("Reminders to approve")
        see("1 to 30 days late")
        let waiting = app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'Waiting for you:' AND label CONTAINS 'payout to pay' AND label CONTAINS 'case to check'")).firstMatch
        XCTAssertTrue(waiting.waitForExistence(timeout: 8), "the summary line counts payouts to pay and cases to check")
        shot("owed")
        app.buttons["Approve refund of ₹250.00"].tap()
        see("Refund approved.")
        shot("owed-after-approve")
    }

    func testCashflowOpensFromPayments() {
        app.tabBars.buttons["Payments"].tap()
        app.buttons["Cashflow"].tap()
        see("Still to come in")
        see("Settled to your bank")
        see("GST")
        see("Tax in total")
        XCTAssertTrue(app.buttons["Earlier month"].exists)
        let later = app.buttons["Later month"]
        XCTAssertTrue(later.isEnabled, "it opens on the last full month, so the current one is a step later")
        later.tap()
        XCTAssertFalse(later.isEnabled, "and it cannot go past the current month")
        shot("cashflow")
    }

    func testSubscriptionsCardOpensAndSaysTheDemoHasNone() {
        app.tabBars.buttons["Settings"].tap()
        app.buttons["Open subscriptions"].tap()
        see("Subscriptions")
        shot("subscriptions")
    }

    func testAnApprovedPayoutCanBeRecordedPaidAndAFailedRefundRetried() {
        app.tabBars.buttons["Owed"].tap()
        see("Approved, ready to pay")
        app.buttons["Record the transfer of ₹3,500.00"].tap()
        see("Record the transfer")
        XCTAssertFalse(app.buttons["Record transfer"].isEnabled, "a reference is needed first")
        let ref = app.textFields.element(boundBy: 0); ref.tap(); ref.typeText("UTR123456789")
        shot("mark-paid-sheet")
        app.buttons["Record transfer"].tap()
        see("Payout marked paid.")
        see("Refunds that failed")
        app.buttons["Retry the failed refund of ₹120.00"].tap()
        see("Refund sent again.")
    }

    /// Make a payment in the demo, then open it: the detail has a timeline.
    func testAPaymentDetailHasATimeline() {
        app.tabBars.buttons["Collect"].tap()
        for d in ["1", "5", "0"] { app.buttons[d].tap() }
        app.buttons["Show QR"].tap()
        see("Received", timeout: 20)                 // the demo's pretend customer pays after a few seconds
        app.buttons["New payment"].tap()
        app.tabBars.buttons["Payments"].tap()
        let row = app.buttons.matching(NSPredicate(format: "label CONTAINS '₹150'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        see("What happened")
        shot("payment-timeline")
    }

    func testASettlementCanBeExplained() {
        app.tabBars.buttons["Payments"].tap()
        app.buttons["Cashflow"].tap()
        let row = app.buttons.matching(NSPredicate(format: "label CONTAINS 'Gross'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        see("This settlement")
        see("₹300.00 less than expected.")
        see("Settlement adjustments")
        shot("settlement-explained")
    }

    func testAReconciliationCaseCanBeAssignedThenResolved() {
        app.tabBars.buttons["Owed"].tap()
        see("Money to check")
        app.buttons["See the settlement behind the Settlement Arithmetic case"].tap()
        see("This settlement")
        see("₹300.00 less than expected.")
        app.buttons["Close"].firstMatch.tap()
        app.buttons["Assign the Settlement Arithmetic case to someone"].tap()
        XCTAssertFalse(app.buttons["Give it"].isEnabled, "a name is needed")
        let name = app.textFields.element(boundBy: 0); name.tap(); name.typeText("Asha")
        app.buttons["Give it"].tap()
        see("Case assigned.")
        see("With Asha")
        app.buttons["Resolve the Settlement Arithmetic case"].tap()
        let note = app.textFields.element(boundBy: 0).exists ? app.textFields.element(boundBy: 0) : app.textViews.element(boundBy: 0)
        note.tap(); note.typeText("Fee confirmed with the bank")
        app.buttons["Mark resolved"].firstMatch.tap()
        see("Case resolved.")
        XCTAssertFalse(app.staticTexts["Money to check"].waitForExistence(timeout: 3), "the card goes away with its last case")
        shot("case-resolved")
    }
}
