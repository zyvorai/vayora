import XCTest

/// The inbox screens against a sandbox that `scripts/seed-sandbox-ios.sh` has filled (a refund and a payout waiting for approval,
/// an open dispute due in two days, a reminder draft). Skipped unless VAYORA_SANDBOX_URL, VAYORA_SANDBOX_KEY and VAYORA_SANDBOX_SEEDED=1
/// are passed to the runner (TEST_RUNNER_ prefix). With one key the person who asked is also the person approving, so the server's
/// separation-of-duties refusal is what these tests expect to see: that is the real behaviour, and it must reach the screen.
final class SeededSandboxTests: XCTestCase {
    var app: XCUIApplication!

    override func setUpWithError() throws {
        let env = ProcessInfo.processInfo.environment
        let url = try XCTUnwrap(env["VAYORA_SANDBOX_URL"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        let key = try XCTUnwrap(env["VAYORA_SANDBOX_KEY"].flatMap { $0.isEmpty ? nil : $0 }, "no sandbox")
        try XCTSkipUnless(env["VAYORA_SANDBOX_SEEDED"] == "1", "sandbox not seeded")
        continueAfterFailure = false
        app = XCUIApplication()
        // a debug build reads these two (a test key only) instead of anyone typing into Settings
        app.launchArguments = ["-demoMode", "NO", "-cashierLocked", "NO", "-uitestServer", url, "-uitestKey", key]
        app.launch()
        app.tabBars.buttons["Owed"].tap()
    }

    func shot(_ name: String) { let a = XCTAttachment(screenshot: app.screenshot()); a.name = name; a.lifetime = .keepAlways; add(a) }
    func see(_ text: String, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(app.staticTexts[text].firstMatch.waitForExistence(timeout: timeout), "expected to see “\(text)”", file: file, line: line)
    }
    func seeContaining(_ part: String, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        let p = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", part)).firstMatch
        XCTAssertTrue(p.waitForExistence(timeout: timeout), "expected text containing “\(part)”", file: file, line: line)
    }

    func testTheInboxShowsTheSeededRows() {
        see("Waiting for your approval"); see("Disputes to answer")   // the draft is approved by testReminderDraftApprovalWorks, so it is not asserted here
        seeContaining("Waiting for you:")   // the summary line, as VoiceOver reads it
        see("Refund · ₹25.00"); see("Payout · ₹1,200.00"); see("₹450.00")
        shot("seeded-owed")
    }

    func testPayoutApprovalByTheRequesterIsRefusedAndTheMessageIsShown() {
        app.buttons["Approve payout of ₹1,200.00"].firstMatch.tap()
        seeContaining("cannot approve")
        shot("seeded-refusal")
        XCTAssertTrue(app.staticTexts["Payout · ₹1,200.00"].exists, "a refused approval must leave the payout in the inbox")
    }

    /// One draft per overdue invoice per 7 days: a sandbox whose draft was already approved cannot make another, so this skips then.
    func testReminderDraftApprovalWorks() throws {
        let approve = app.buttons["Approve reminder for INV-1001"].firstMatch
        try XCTSkipUnless(approve.waitForExistence(timeout: 12), "no reminder draft waiting (the dunning policy waits 7 days between reminders)")
        approve.tap()
        see("Reminder approved.")
        XCTAssertFalse(app.buttons["Approve reminder for INV-1001"].waitForExistence(timeout: 3), "an approved draft leaves the inbox")
    }

    func testEvidenceNoteIsSavedOnTheRealDispute() {
        app.buttons["Add evidence to the ₹450.00 dispute"].firstMatch.tap()
        let note = app.textViews.firstMatch.exists ? app.textViews.firstMatch : app.textFields.element(boundBy: 0)
        note.tap(); note.typeText("Delivered with a signed receipt")
        app.buttons["Save"].tap()
        see("Evidence added.")
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "[0-9]+ notes? added")).firstMatch.waitForExistence(timeout: 10))
        shot("seeded-evidence")
    }

    /// Needs an approved payout (the approver test makes one); skipped when there is none.
    func testAnApprovedPayoutIsRecordedPaidOnTheRealServer() throws {
        let b = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Record the transfer of'")).firstMatch
        try XCTSkipUnless(b.waitForExistence(timeout: 12), "no approved payout in this sandbox")
        b.tap()
        let ref = app.textFields.element(boundBy: 0); ref.tap(); ref.typeText("UTR\(Int(Date().timeIntervalSince1970))")
        app.buttons["Record transfer"].tap()
        see("Payout marked paid.")
        shot("seeded-paid")
    }
}
