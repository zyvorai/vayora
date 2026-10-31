import XCTest
@testable import VayoraKit

final class UPIPayloadTests: XCTestCase {
    func testReadsAVayoraPayload() throws {
        let p = try UPIPayload.parse("upi://pay?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am=499.00&cu=INR&tn=att_1")
        XCTAssertEqual(p.payee, "sandbox@zyvor-sim")
        XCTAssertEqual(p.name, "Zyvor Sandbox")
        XCTAssertEqual(p.amount, Money(minor: 49900))
        XCTAssertEqual(p.note, "att_1")
    }
    func testAmountIsOptional() throws {
        XCTAssertNil(try UPIPayload.parse("upi://pay?pa=shop@okbank").amount)
    }
    func testRefusesHostileInput() {
        let cases: [(String, UPIPayload.Failure)] = [
            ("https://evil.example/pay?pa=a@b", .notUPI),
            ("upi://mandate?pa=a@b", .notUPI),
            ("upi://pay?pn=NoPayee", .missingPayee),
            ("upi://pay?pa=no-at-sign", .badPayee),
            ("upi://pay?pa=a@b@c", .badPayee),
            ("upi://pay?pa=a@b&am=-5", .badAmount),
            ("upi://pay?pa=a@b&am=1.239", .badAmount),
            ("upi://pay?pa=a@b&am=10&am=1", .badAmount),
            ("upi://pay?pa=a@b&cu=USD", .unsupportedCurrency),
        ]
        for (text, failure) in cases {
            XCTAssertThrowsError(try UPIPayload.parse(text), text) { XCTAssertEqual($0 as? UPIPayload.Failure, failure, text) }
        }
    }
}
