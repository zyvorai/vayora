import XCTest
@testable import VayoraKit

final class RefundStateTests: XCTestCase {
    private func attempts(_ json: String) throws -> [AttemptInfo] {
        try JSONDecoder().decode([AttemptInfo].self, from: Data(json.utf8))
    }

    func testFullRefundLeavesNothingToRefund() throws {
        let s = RefundState(attempts: try attempts(#"[{"id":"a","status":"captured","amount_minor":49900,"refunded_minor":49900,"reserved_refund_minor":0}]"#))
        XCTAssertEqual(s?.remainingMinor, 0)
        XCTAssertEqual(s?.isFullyRefunded, true)
    }
    func testPartialRefundAndInProgressRefundBothCount() throws {
        let s = RefundState(attempts: try attempts(#"[{"id":"a","status":"captured","amount_minor":49900,"refunded_minor":10000,"reserved_refund_minor":5000}]"#))
        XCTAssertEqual(s?.remainingMinor, 34900)
        XCTAssertEqual(s?.isPartlyRefunded, true)
        XCTAssertEqual(s?.isFullyRefunded, false)
    }
    func testUnpaidPaymentHasNoRefundState() throws {
        XCTAssertNil(RefundState(attempts: try attempts(#"[{"id":"a","status":"pending","amount_minor":49900}]"#)))
    }
    func testOlderServersWithoutRefundFieldsReadAsNotRefunded() throws {
        let s = RefundState(attempts: try attempts(#"[{"id":"a","status":"captured","amount_minor":49900}]"#))
        XCTAssertEqual(s?.refundedMinor, 0)
        XCTAssertEqual(s?.remainingMinor, 49900)
    }
}
