import XCTest

/// Walks the payer app in its built-in demo (test money, no server): the tabs, then a payment from a pasted UPI link.
/// Screenshots are attached to the result. A smoke test, not a layout or VoiceOver check.
final class PayerWalkthroughTests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-onboarded", "YES", "-demoMode", "YES", "-faceIDLock", "NO"]
        app.launch()
    }

    func shot(_ name: String) { let a = XCTAttachment(screenshot: app.screenshot()); a.name = name; a.lifetime = .keepAlways; add(a) }

    func testEveryTabOpens() {
        for tab in ["Home", "Payees", "Bills", "History", "Settings"] {
            let b = app.tabBars.buttons[tab]
            XCTAssertTrue(b.waitForExistence(timeout: 8), "tab \(tab)")
            b.tap()
            shot("tab-\(tab.lowercased())")
        }
    }

    /// Pays a pasted link (slide to pay, then the test PIN), then checks the payment shows in History and, by name, in "Who you paid most"
    /// on the weekly recap. Each run uses its own payee: the Simulator keeps the app's data between runs, and a known payee would not be flagged.
    func testAPastedLinkCanBePaidAndShowsInHistoryAndTheRecap() {
        let stamp = String(Int(Date().timeIntervalSince1970))
        let name = "Shop \(stamp)", vpa = "shop\(stamp)@vayora"
        app.buttons["Paste link"].tap()
        let field = app.textFields["upi://pay?pa=…"]
        XCTAssertTrue(field.waitForExistence(timeout: 8))
        field.tap(); field.typeText("upi://pay?pa=\(vpa)&pn=Shop%20\(stamp)&am=100.00&cu=INR")
        app.buttons["Continue"].tap()
        XCTAssertTrue(app.staticTexts[name].waitForExistence(timeout: 8))
        XCTAssertTrue(app.staticTexts["First time paying this UPI ID"].exists, "a new payee must be flagged")
        shot("pay-sheet")
        let slider = app.buttons["Slide to pay"]
        XCTAssertTrue(slider.waitForExistence(timeout: 5))
        let from = slider.coordinate(withNormalizedOffset: CGVector(dx: 0.06, dy: 0.5))
        let to = slider.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5))
        from.press(forDuration: 0.3, thenDragTo: to)
        XCTAssertTrue(app.staticTexts["Enter your test PIN"].waitForExistence(timeout: 8), "sliding asks for the PIN")
        for d in ["1", "2", "3", "4"] { app.buttons[d].tap() }     // the test PIN, 1234 by default
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] 'paid' OR label CONTAINS[c] 'sent'")).firstMatch.waitForExistence(timeout: 12), "a result screen after paying")
        shot("paid")
        app.buttons["Done"].firstMatch.tap()
        app.tabBars.buttons["History"].tap()
        XCTAssertTrue(app.staticTexts[name].firstMatch.waitForExistence(timeout: 8), "the payment is in History")
        app.buttons["Week"].tap()
        XCTAssertTrue(app.staticTexts["Who you paid most"].waitForExistence(timeout: 8), "the recap lists who was paid")
        XCTAssertTrue(app.staticTexts[name].firstMatch.exists, "by the name the person recognises")
        XCTAssertFalse(app.staticTexts[vpa].exists, "not by the UPI id")
        shot("recap-payees")
    }

    func testPasteALinkShowsThePaymentSheet() {
        app.buttons["Paste link"].tap()
        let field = app.textFields["upi://pay?pa=…"]
        XCTAssertTrue(field.waitForExistence(timeout: 8))
        field.tap(); field.typeText("upi://pay?pa=demo-shop@vayora&pn=Demo%20Shop&am=100.00&cu=INR")
        app.buttons["Continue"].tap()
        XCTAssertTrue(app.staticTexts["Demo Shop"].waitForExistence(timeout: 8))
    }
}
