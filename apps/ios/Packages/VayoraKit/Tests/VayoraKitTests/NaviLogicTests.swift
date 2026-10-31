import XCTest
@testable import VayoraKit

final class RequestPayloadTests: XCTestCase {
    func testRoundTripKeepsPaiseExact() throws {
        let s = RequestPayload.make(payee: "asha@okzyvor", name: "Asha K", amount: Money(minor: 49950), note: "Dinner split")
        let p = try RequestPayload.parse(s)
        XCTAssertEqual(p.payee, "asha@okzyvor"); XCTAssertEqual(p.name, "Asha K"); XCTAssertEqual(p.amount?.minor, 49950); XCTAssertEqual(p.note, "Dinner split")
    }
    func testRefusesNonRequestsMissingAmountsAndBadPayees() {
        XCTAssertThrowsError(try RequestPayload.parse("upi://pay?pa=a@b&am=5")) { XCTAssertEqual($0 as? RequestPayload.Failure, .notRequest) }
        XCTAssertThrowsError(try RequestPayload.parse("vayora://setup?server=x")) { XCTAssertEqual($0 as? RequestPayload.Failure, .notRequest) }
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=asha@okzyvor")) { XCTAssertEqual($0 as? RequestPayload.Failure, .missingAmount) }
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=not-a-vpa&am=5")) { XCTAssertEqual($0 as? RequestPayload.Failure, .invalid(.badPayee)) }
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=a@b&am=1.234")) { XCTAssertEqual($0 as? RequestPayload.Failure, .invalid(.badAmount)) }
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=a@b&am=5&am=6")) { XCTAssertEqual($0 as? RequestPayload.Failure, .invalid(.badAmount)) }
    }
}

final class PayeeTests: XCTestCase {
    func testInputAcceptsUPIIdsAndIndianMobileNumbers() {
        XCTAssertEqual(PayeeInput.vpa(from: " Asha@OkZyvor "), "asha@okzyvor")
        XCTAssertEqual(PayeeInput.vpa(from: "98765 43210"), "9876543210@zyvor-test")
        XCTAssertEqual(PayeeInput.vpa(from: "+91 98765-43210"), "9876543210@zyvor-test")
        XCTAssertEqual(PayeeInput.vpa(from: "098765 43210"), "9876543210@zyvor-test")
    }
    func testInputRefusesEverythingElse() {
        for bad in ["", "hello", "12345", "5876543210", "98765432101", "a@b@c", "a b@c", "٩٨٧٦٥٤٣٢١٠"] { XCTAssertNil(PayeeInput.vpa(from: bad), bad) }
    }
    func testBookDeDuplicatesAndOrdersFavouritesThenRecent() {
        var b = PayeeBook()
        let t0 = Date(timeIntervalSince1970: 1_000), t1 = Date(timeIntervalSince1970: 2_000)
        XCTAssertTrue(b.save(name: "Asha", input: "asha@okzyvor", paidAt: t0))
        XCTAssertTrue(b.save(name: "Ravi", input: "9876543210", paidAt: t1))
        XCTAssertTrue(b.save(name: "Asha K", input: "ASHA@okzyvor"))
        XCTAssertEqual(b.all.count, 2); XCTAssertEqual(b.all.first { $0.vpa == "asha@okzyvor" }?.name, "Asha K")
        XCTAssertEqual(b.sorted.map(\.name), ["Ravi", "Asha K"])
        b.toggleFavourite("asha@okzyvor")
        XCTAssertEqual(b.sorted.map(\.name), ["Asha K", "Ravi"])
        XCTAssertFalse(b.save(name: "x", input: "nope"))
        b.remove("asha@okzyvor"); XCTAssertEqual(b.all.count, 1)
    }
}

final class SmallPaymentTests: XCTestCase {
    let p = SmallPaymentPolicy()
    func testLimitsAreInclusiveAtThePaymentAndTheDay() {
        XCTAssertFalse(p.needsPIN(amount: Money(minor: 20_000), noPINSpentTodayMinor: 0, enabled: true), "₹200.00 is allowed")
        XCTAssertTrue(p.needsPIN(amount: Money(minor: 20_001), noPINSpentTodayMinor: 0, enabled: true), "₹200.01 is not")
        XCTAssertFalse(p.needsPIN(amount: Money(minor: 20_000), noPINSpentTodayMinor: 80_000, enabled: true), "reaches exactly ₹1,000")
        XCTAssertTrue(p.needsPIN(amount: Money(minor: 20_000), noPINSpentTodayMinor: 80_001, enabled: true), "would pass ₹1,000")
    }
    func testOffMeansPINAndOddAmountsNeedPIN() {
        XCTAssertTrue(p.needsPIN(amount: Money(minor: 100), noPINSpentTodayMinor: 0, enabled: false))
        XCTAssertTrue(p.needsPIN(amount: Money(minor: 0), noPINSpentTodayMinor: 0, enabled: true))
        XCTAssertTrue(p.needsPIN(amount: Money(minor: 100, currency: "USD"), noPINSpentTodayMinor: 0, enabled: true))
    }
}

final class SpendingTests: XCTestCase {
    var cal: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; return c }
    func day(_ y: Int, _ m: Int, _ d: Int) -> Date { cal.date(from: DateComponents(year: y, month: m, day: d, hour: 12))! }

    func testCategoryGuessIsOnlyAGuess() {
        XCTAssertEqual(SpendCategory.guess(name: "Chai Point", note: "", transfer: false), .food)
        XCTAssertEqual(SpendCategory.guess(name: "Asha", note: "electricity bill", transfer: false), .bills)
        XCTAssertEqual(SpendCategory.guess(name: "Chai Point", note: "", transfer: true), .transfers)
        XCTAssertEqual(SpendCategory.guess(name: "Zyvor Sandbox", note: "", transfer: false), .other)
    }
    func testMonthSummaryCountsOnlyPaidPaymentsInThisMonth() {
        let now = day(2026, 10, 31)
        let e = [SpendEntry(amountMinor: 50_000, category: .food, date: day(2026, 10, 2), paid: true),
                 SpendEntry(amountMinor: 20_000, category: .food, date: day(2026, 10, 30), paid: true),
                 SpendEntry(amountMinor: 99_999, category: .bills, date: day(2026, 10, 5), paid: false),
                 SpendEntry(amountMinor: 70_000, category: .travel, date: day(2026, 9, 30), paid: true),
                 SpendEntry(amountMinor: 10_000, category: .bills, date: day(2026, 10, 9), paid: true)]
        let s = MonthSummary.byCategory(e, now: now, calendar: cal)
        XCTAssertEqual(s.map(\.category), [.food, .bills]); XCTAssertEqual(s.map(\.minor), [70_000, 10_000])
        XCTAssertEqual(MonthSummary.total(e, now: now, calendar: cal), 80_000)
    }
    func testReminderIsAlwaysInTheFutureAndClampsTheDay() {
        let r = BillReminder(name: "Rent", amountMinor: 1_200_000, dayOfMonth: 31)
        XCTAssertEqual(r.dayOfMonth, 28)
        let due = r.nextDue(after: day(2026, 10, 31), calendar: cal)
        XCTAssertEqual(cal.component(.month, from: due), 11); XCTAssertEqual(cal.component(.day, from: due), 28); XCTAssertEqual(cal.component(.hour, from: due), 9)
        XCTAssertGreaterThan(BillReminder(name: "x", amountMinor: 1, dayOfMonth: 5).nextDue(after: day(2026, 10, 5), calendar: cal), day(2026, 10, 5))
        XCTAssertEqual(cal.component(.month, from: BillReminder(name: "x", amountMinor: 1, dayOfMonth: 28).nextDue(after: day(2026, 12, 1), calendar: cal)), 12)
    }
}
