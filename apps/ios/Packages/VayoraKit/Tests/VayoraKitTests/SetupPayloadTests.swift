import XCTest
@testable import VayoraKit

final class SetupPayloadTests: XCTestCase {
    func testParsesServerAndTestKey() throws {
        let p = try SetupPayload.parse("vayora://setup?server=http%3A%2F%2F192.168.1.10%3A18080&key=zk_test_abc123")
        XCTAssertEqual(p.server.absoluteString, "http://192.168.1.10:18080")
        XCTAssertEqual(p.apiKey, "zk_test_abc123")
    }
    func testRefusesLiveKeysAndOddInput() {
        XCTAssertThrowsError(try SetupPayload.parse("vayora://setup?server=https%3A%2F%2Fa.example&key=zk_live_x")) { XCTAssertEqual($0 as? SetupPayload.Failure, .liveKeyRefused) }
        XCTAssertThrowsError(try SetupPayload.parse("upi://pay?pa=a@b")) { XCTAssertEqual($0 as? SetupPayload.Failure, .notSetup) }
        XCTAssertThrowsError(try SetupPayload.parse("vayora://setup?server=ftp%3A%2F%2Fx&key=zk_test_a")) { XCTAssertEqual($0 as? SetupPayload.Failure, .badServer) }
        XCTAssertThrowsError(try SetupPayload.parse("vayora://setup?server=http%3A%2F%2Fa.b&key=")) { XCTAssertEqual($0 as? SetupPayload.Failure, .missingKey) }
        XCTAssertThrowsError(try SetupPayload.parse("vayora://setup?server=http%3A%2F%2Fa.b&key=zk_test_a&key=zk_test_b")) { XCTAssertEqual($0 as? SetupPayload.Failure, .missingKey) }
    }
}
