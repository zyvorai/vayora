import XCTest
@testable import VayoraKit

final class MoneyTests: XCTestCase {
    func testParsesPlainDecimals() {
        XCTAssertEqual(Money(decimal: "499")?.minor, 49900)
        XCTAssertEqual(Money(decimal: "499.5")?.minor, 49950)
        XCTAssertEqual(Money(decimal: "0.05")?.minor, 5)
    }
    func testRejectsAnythingElse() {
        for bad in ["", "0", "0.00", "-5", "1.234", "1e3", "abc", ".5", "5.", "١٢٣", "1,000", " 5", "99999999999"] {
            XCTAssertNil(Money(decimal: bad), bad)
        }
    }
    func testDisplayUsesIndianGrouping() {
        XCTAssertEqual(Money(minor: 49900).display, "₹499.00")
        XCTAssertEqual(Money(minor: 123_456_00).display, "₹1,23,456.00")
        XCTAssertEqual(Money(minor: 10_00_00_000_00).display, "₹10,00,00,000.00")
    }
    func testDecimalStringKeepsPaise() { XCTAssertEqual(Money(minor: 5).decimalString, "0.05") }
}
