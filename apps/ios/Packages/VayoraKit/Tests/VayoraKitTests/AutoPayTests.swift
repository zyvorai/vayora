import XCTest
@testable import VayoraKit

final class MandatePayloadTests: XCTestCase {
    let good = "upi://mandate?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am=5000.00&cu=INR&tn=mnd_01a192e1d0ca&recur=monthly&start=2026-11-01&end=2027-11-01"

    func testParsesARealLink() throws {
        let p = try MandatePayload.parse(good)
        XCTAssertEqual(p.payee, "sandbox@zyvor-sim"); XCTAssertEqual(p.name, "Zyvor Sandbox")
        XCTAssertEqual(p.cap.minor, 500_000); XCTAssertEqual(p.mandateID, "mnd_01a192e1d0ca")
        XCTAssertEqual(p.frequency, .monthly); XCTAssertNotNil(p.start); XCTAssertNotNil(p.end)
        XCTAssertEqual(MandateFrequency.monthly.title, "Every month")
    }

    func testRefusesAnythingThatIsNotAWellFormedMandate() {
        func fail(_ s: String, _ f: MandatePayload.Failure, line: UInt = #line) {
            XCTAssertThrowsError(try MandatePayload.parse(s), line: line) { XCTAssertEqual($0 as? MandatePayload.Failure, f, line: line) }
        }
        fail("upi://pay?pa=a@b&am=5&tn=x", .notMandate)
        fail("https://evil.example/mandate?pa=a@b", .notMandate)
        fail("upi://mandate?am=5&tn=mnd_abcd&recur=monthly", .missingPayee)
        fail("upi://mandate?pa=nope&am=5&tn=mnd_abcd&recur=monthly", .badPayee)
        fail("upi://mandate?pa=a@b&am=0&tn=mnd_abcd&recur=monthly", .badAmount)
        fail("upi://mandate?pa=a@b&am=1.234&tn=mnd_abcd&recur=monthly", .badAmount)
        fail("upi://mandate?pa=a@b&tn=mnd_abcd&recur=monthly", .badAmount)
        fail("upi://mandate?pa=a@b&am=5&cu=USD&tn=mnd_abcd&recur=monthly", .unsupportedCurrency)
        fail("upi://mandate?pa=a@b&am=5&recur=monthly", .badMandateID)
        fail("upi://mandate?pa=a@b&am=5&tn=../../etc&recur=monthly", .badMandateID)
        fail("upi://mandate?pa=a@b&am=5&tn=mnd_abcd&recur=hourly", .badFrequency)
        fail("upi://mandate?pa=a@b&am=5&tn=mnd_abcd", .badFrequency)
        fail("upi://mandate?pa=a@b&am=5&tn=mnd_abcd&recur=monthly&end=2027-02-30", .badDate)
        fail("upi://mandate?pa=a@b&am=5&tn=mnd_abcd&recur=monthly&start=2027-02-01&end=2027-01-01", .badDate)
        fail("upi://mandate?pa=a@b&am=5&am=6&tn=mnd_abcd&recur=monthly", .duplicateParameter)
    }

    func testMandateIDsCannotReshapeAURLPath() {
        for bad in ["", "abc", "mnd/../x", "mnd_abcd?x=1", "mnd abcd", "mnd_abcd\n", String(repeating: "a", count: 65), "mnd_ａｂｃｄ"] {
            XCTAssertFalse(MandatePayload.isSafeID(bad), bad)
        }
        XCTAssertTrue(MandatePayload.isSafeID("mnd_01a192e1d0ca75e5a26a19ea96ade73f"))
    }

    func testADateDisplaysAsTheDayItNamesInAnyTimeZone() throws {
        for tz in ["Asia/Kolkata", "America/Los_Angeles", "Pacific/Auckland", "Pacific/Kiritimati", "Etc/GMT+12"] {
            var cal = Calendar(identifier: .gregorian); cal.timeZone = TimeZone(identifier: tz)!
            let d = try MandatePayload.date("2026-12-31", calendar: cal)
            let c = cal.dateComponents([.month, .day], from: d)
            XCTAssertEqual([c.month, c.day], [12, 31], tz)
        }
        XCTAssertThrowsError(try MandatePayload.date("2027-02-30"))
        XCTAssertThrowsError(try MandatePayload.date("27-1-1"))
    }
}

final class AutoPayLedgerTests: XCTestCase {
    func debit(_ id: String, _ amt: Int64) -> PayerDebit { PayerDebit(attemptID: id, amountMinor: amt, at: "2026-11-01T10:00:00Z") }

    func testEachDebitIsAppliedExactlyOnceAcrossRepeatedPolls() {
        var applied = Set<String>()
        let first = AutoPayLedger.newDebits(applied: applied, reported: [debit("a", 100), debit("b", 250)])
        XCTAssertEqual(first.map(\.attemptID), ["a", "b"]); applied.formUnion(first.map(\.attemptID))
        XCTAssertTrue(AutoPayLedger.newDebits(applied: applied, reported: [debit("a", 100), debit("b", 250)]).isEmpty, "a second poll adds nothing")
        let later = AutoPayLedger.newDebits(applied: applied, reported: [debit("a", 100), debit("b", 250), debit("c", 75)])
        XCTAssertEqual(later.map(\.attemptID), ["c"])
        XCTAssertEqual(AutoPayLedger.total(first).minor, 350)
    }

    func testARepeatInsideOneReportIsAppliedOnce() {
        XCTAssertEqual(AutoPayLedger.newDebits(applied: [], reported: [debit("a", 100), debit("a", 100)]).count, 1)
    }

    func testTheViewDecodesTheServerShape() throws {
        let json = #"{"mandate_id":"mnd_1","status":"active","max_amount_minor":500000,"debits":[{"attempt_id":"att_1","amount_minor":250000,"at":"2026-11-01T10:00:00+00:00"}]}"#
        let v = try JSONDecoder().decode(PayerMandateView.self, from: Data(json.utf8))
        XCTAssertEqual(v.status, "active"); XCTAssertEqual(v.debits.first?.money.minor, 250_000)
    }

    func testAMandateAndItsDebitDecode() throws {
        let m = #"{"id":"mnd_1","customer_id":"cus_1","currency":"INR","max_amount_minor":500000,"frequency":"half_yearly","start_date":"2026-11-01","end_date":"2027-11-01","status":"active","provider":"sim","authorization":{"type":"upi_mandate","uri":"upi://mandate?pa=a@b"},"failure_reason":null,"revoked_by":null,"created_at":"2026-11-01T00:00:00Z"}"#
        let mi = try JSONDecoder().decode(MandateInfo.self, from: Data(m.utf8))
        XCTAssertEqual(mi.frequencyTitle, "Every 6 months"); XCTAssertTrue(mi.isLive); XCTAssertEqual(mi.cap.minor, 500_000)
        let d = #"{"id":"mdb_1","mandate_id":"mnd_1","amount_minor":1000,"currency":"INR","due_date":"2026-11-05","notified_at":null,"invoice_id":null,"state":"scheduled","attempt_id":null,"retry_no":0,"failure_reason":null,"created_at":"2026-11-01T00:00:00Z"}"#
        let di = try JSONDecoder().decode(MandateDebitInfo.self, from: Data(d.utf8))
        XCTAssertEqual(di.state, "scheduled"); XCTAssertEqual(di.money.minor, 1000)
    }
}
