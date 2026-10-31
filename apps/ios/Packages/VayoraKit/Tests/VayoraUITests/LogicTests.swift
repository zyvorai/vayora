import XCTest
@testable import VayoraUI
@testable import VayoraKit

final class AmountEntryTests: XCTestCase {
    private func typed(_ keys: [Character]) -> AmountEntry {
        var e = AmountEntry()
        for k in keys { if k == "." { e.point() } else if k == "<" { e.delete() } else if let d = k.wholeNumberValue { e.digit(d) } }
        return e
    }
    func testBuildsAnAmountInPaiseNeverFloat() {
        XCTAssertEqual(typed(Array("499.5")).money?.minor, 49950)
        XCTAssertEqual(typed(Array("499")).money?.minor, 49900)
        XCTAssertEqual(typed(Array("0.07")).money?.minor, 7)
    }
    func testRefusesThirdDecimalAndLeadingZeros() {
        XCTAssertEqual(typed(Array("1.234")).text, "1.23")
        XCTAssertEqual(typed(Array("007")).text, "7")
        XCTAssertEqual(typed(Array(".5")).text, "0.5")
        XCTAssertEqual(typed(Array("1..2")).text, "1.2")
    }
    func testCapsTheIntegerPartAndNeedsAPositiveAmount() {
        XCTAssertEqual(typed(Array("123456789")).text, "1234567")
        XCTAssertNil(AmountEntry().money)
        XCTAssertNil(typed(Array("0")).money)
        XCTAssertNil(typed(Array("5.")).money, "a trailing point is not an amount yet")
    }
}

final class SlideAndPinTests: XCTestCase {
    func testSlideCompletesOnlyNearTheEnd() {
        XCTAssertEqual(SlideModel.outcome(progress: 0.5), .reset)
        XCTAssertEqual(SlideModel.outcome(progress: 0.91), .reset)
        XCTAssertEqual(SlideModel.outcome(progress: 0.92), .complete)
        XCTAssertEqual(SlideModel.outcome(progress: 1), .complete)
    }
    func testTicksFireOncePerTenthAndStayInRange() {
        XCTAssertEqual(SlideModel.tickStep(progress: 0), 0)
        XCTAssertEqual(SlideModel.tickStep(progress: 0.34), 3)
        XCTAssertEqual(SlideModel.tickStep(progress: 1), 10)
        XCTAssertEqual(SlideModel.tickStep(progress: 4), 10)
        XCTAssertEqual(SlideModel.tickStep(progress: -1), 0)
    }
    func testPinFillsDeletesAndRefusesExtraDigits() {
        var p = PinEntry()
        [1, 2, 3, 4, 5].forEach { p.press($0) }
        XCTAssertEqual(p.code, "1234"); XCTAssertTrue(p.isFull)
        p.delete(); XCTAssertEqual(p.code, "123"); XCTAssertFalse(p.isFull)
        p.press(99); XCTAssertEqual(p.code, "123", "not a digit")
        p.clear(); XCTAssertEqual(p.code, "")
    }
}
