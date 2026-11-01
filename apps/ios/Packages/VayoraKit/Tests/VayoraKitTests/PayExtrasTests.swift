import XCTest
@testable import VayoraKit

final class BudgetTests: XCTestCase {
    var cal: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; return c }
    func date(_ s: String) -> Date { let f = ISO8601DateFormatter(); f.timeZone = cal.timeZone; f.formatOptions = [.withFullDate]; return cal.date(bySettingHour: 12, minute: 0, second: 0, of: f.date(from: s)!)! }

    func testLevels() {
        let b = Budgets(limits: [.food: 100_00])
        func lvl(_ spent: Int64) -> BudgetStatus.Level {
            BudgetCheck.statuses([SpendEntry(amountMinor: spent, category: .food, date: date("2026-10-10"), paid: true)], budgets: b, now: date("2026-10-20"), calendar: cal).first!.level
        }
        XCTAssertEqual(lvl(79_99), .ok); XCTAssertEqual(lvl(80_00), .near); XCTAssertEqual(lvl(100_00), .near); XCTAssertEqual(lvl(100_01), .over)
    }
    func testOnlyThisMonthAndPaidCount() {
        let b = Budgets(limits: [.food: 100_00])
        let e = [SpendEntry(amountMinor: 90_00, category: .food, date: date("2026-09-30"), paid: true),
                 SpendEntry(amountMinor: 90_00, category: .food, date: date("2026-10-02"), paid: false),
                 SpendEntry(amountMinor: 10_00, category: .food, date: date("2026-10-03"), paid: true)]
        let s = BudgetCheck.statuses(e, budgets: b, now: date("2026-10-20"), calendar: cal)
        XCTAssertEqual(s.first?.spentMinor, 10_00); XCTAssertEqual(s.first?.leftMinor, 90_00)
    }
    func testUnwatchedCategoryAndZeroLimitIgnored() {
        var b = Budgets(); b.set(.travel, minor: 0); b.set(.bills, minor: -5)
        XCTAssertTrue(BudgetCheck.statuses([], budgets: b, now: Date()).isEmpty)
    }
    func testWorstFirst() {
        let b = Budgets(limits: [.food: 100_00, .bills: 100_00])
        let e = [SpendEntry(amountMinor: 20_00, category: .food, date: date("2026-10-02"), paid: true), SpendEntry(amountMinor: 95_00, category: .bills, date: date("2026-10-02"), paid: true)]
        XCTAssertEqual(BudgetCheck.statuses(e, budgets: b, now: date("2026-10-20"), calendar: cal).map(\.category), [.bills, .food])
    }
    func testWarningBeforePayingNeverForOkOrUnwatched() {
        let b = Budgets(limits: [.food: 100_00])
        let e = [SpendEntry(amountMinor: 50_00, category: .food, date: date("2026-10-02"), paid: true)]
        XCTAssertNil(BudgetCheck.warning(adding: 10_00, to: .food, entries: e, budgets: b, now: date("2026-10-20"), calendar: cal))
        XCTAssertEqual(BudgetCheck.warning(adding: 60_00, to: .food, entries: e, budgets: b, now: date("2026-10-20"), calendar: cal)?.level, .over)
        XCTAssertNil(BudgetCheck.warning(adding: 999_00, to: .travel, entries: e, budgets: b, now: date("2026-10-20"), calendar: cal))
    }
    func testBudgetsRoundTrip() throws {
        let b = Budgets(limits: [.food: 5000])
        XCTAssertEqual(try JSONDecoder().decode(Budgets.self, from: JSONEncoder().encode(b)), b)
    }
}

final class HistoryToolsTests: XCTestCase {
    var cal: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; return c }
    func row(_ n: String, _ t: TimeInterval, paid: Bool = true, cat: SpendCategory = .food, note: String = "") -> HistoryRow {
        HistoryRow(date: Date(timeIntervalSince1970: t), name: n, payee: n.lowercased() + "@bank", amountMinor: 12_50, paid: paid, category: cat, note: note, kind: "")
    }
    func testFilters() {
        let rows = [row("Chai Point", 1_700_000_000), row("Metro", 1_700_100_000, cat: .travel), row("Rent", 1_700_200_000, paid: false, cat: .bills)]
        var f = HistoryFilter(); XCTAssertFalse(f.isActive); XCTAssertEqual(f.apply(rows, calendar: cal).count, 3)
        f.text = "chai"; XCTAssertEqual(f.apply(rows, calendar: cal).map(\.name), ["Chai Point"])
        f = HistoryFilter(); f.category = .travel; XCTAssertEqual(f.apply(rows, calendar: cal).map(\.name), ["Metro"])
        f = HistoryFilter(); f.outcome = .failed; XCTAssertEqual(f.apply(rows, calendar: cal).map(\.name), ["Rent"])
        f = HistoryFilter(); f.outcome = .paid; XCTAssertEqual(f.apply(rows, calendar: cal).count, 2)
    }
    func testDateRangeIsInclusiveOfBothDays() {
        let day = Date(timeIntervalSince1970: 1_700_100_000)
        let rows = [row("A", day.timeIntervalSince1970 - 86_400 * 2), row("B", day.timeIntervalSince1970), row("C", day.timeIntervalSince1970 + 86_400 * 2)]
        var f = HistoryFilter(); f.from = day; f.to = day
        XCTAssertEqual(f.apply(rows, calendar: cal).map(\.name), ["B"])
    }
    func testCSVEscapingAndFormulaGuard() {
        let r = HistoryRow(date: Date(timeIntervalSince1970: 1_700_000_000), name: "=HYPERLINK(\"x\")", payee: "a@b", amountMinor: 1_234_50, paid: true, category: .other, note: "tea, \"hot\"\nline", kind: "")
        let csv = HistoryCSV.make([r], calendar: cal)
        let lines = csv.split(separator: "\n", omittingEmptySubsequences: true)
        XCTAssertEqual(lines.count, 2)
        XCTAssertTrue(csv.contains("\"'=HYPERLINK(\"\"x\"\")\""))
        XCTAssertTrue(csv.contains("\"tea, \"\"hot\"\" line\""))
        XCTAssertTrue(csv.contains(",1234.50,paid,other,"))
    }
    func testCSVNewestFirstWithHeader() {
        let csv = HistoryCSV.make([row("Old", 1_600_000_000), row("New", 1_700_000_000)], calendar: cal)
        XCTAssertTrue(csv.hasPrefix("date,name,payee,amount_inr,status,category,kind,note\n"))
        XCTAssertLessThan(csv.range(of: "New")!.lowerBound, csv.range(of: "Old")!.lowerBound)
    }
}

final class OpenRequestTests: XCTestCase {
    func testOpenRequestParsesWithoutAmount() throws {
        let p = try RequestPayload.parse(RequestPayload.makeOpen(payee: "asha@okzyvor", name: "Asha K"))
        XCTAssertEqual(p.payee, "asha@okzyvor"); XCTAssertNil(p.amount)
    }
    func testRequestWithoutAmountAndWithoutOpenStillRefused() {
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=asha@okzyvor&pn=A")) { XCTAssertEqual($0 as? RequestPayload.Failure, .missingAmount) }
    }
    func testOpenFlagRepeatedOrOtherValueDoesNotOpenIt() {
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=asha@okzyvor&pn=A&open=1&open=1"))
        XCTAssertThrowsError(try RequestPayload.parse("vayora://request?pa=asha@okzyvor&pn=A&open=yes"))
    }
    func testOpenWithAmountKeepsTheAmount() throws {
        let p = try RequestPayload.parse("vayora://request?pa=asha@okzyvor&pn=A&am=10.00&open=1")
        XCTAssertEqual(p.amount?.minor, 1000)
    }
}

final class MerchantToolsTests: XCTestCase {
    var cal: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; return c }
    func testTipRoundsHalfUpAndGuards() {
        XCTAssertEqual(Tip.amount(base: 500_00, percent: 10), 50_00)
        XCTAssertEqual(Tip.amount(base: 1, percent: 10), 0)       // 0.1 paisa
        XCTAssertEqual(Tip.amount(base: 5, percent: 10), 1)       // 0.5 paisa rounds up
        XCTAssertEqual(Tip.amount(base: 333_33, percent: 15), 50_00)
        XCTAssertEqual(Tip.amount(base: 0, percent: 10), 0); XCTAssertEqual(Tip.amount(base: -5, percent: 10), 0); XCTAssertEqual(Tip.amount(base: 100, percent: 0), 0)
    }
    func testHourlyOnlyThatDayAndHour() {
        let c = cal
        let day = c.date(from: DateComponents(year: 2026, month: 10, day: 31, hour: 12))!
        func at(_ d: Int, _ h: Int) -> Date { c.date(from: DateComponents(year: 2026, month: 10, day: d, hour: h, minute: 30))! }
        let t = HourlyTotals.make([(at(31, 9), 100), (at(31, 9), 50), (at(31, 23), 7), (at(30, 9), 999), (at(1, 0), 5)], day: day, calendar: c)
        XCTAssertEqual(t.count, 24); XCTAssertEqual(t[9], 150); XCTAssertEqual(t[23], 7); XCTAssertEqual(t.reduce(0, +), 157)
    }
    func testWindowIsAtLeastSixWideAndInRange() {
        var t = [Int64](repeating: 0, count: 24); t[10] = 5
        let w = HourlyTotals.window(t); XCTAssertGreaterThanOrEqual(w.count, 6); XCTAssertTrue(w.contains(10)); XCTAssertTrue(w.lowerBound >= 0 && w.upperBound <= 23)
        t = [Int64](repeating: 0, count: 24); t[0] = 1
        let e = HourlyTotals.window(t); XCTAssertEqual(e.lowerBound, 0); XCTAssertGreaterThanOrEqual(e.count, 6)
        t[23] = 1; XCTAssertEqual(HourlyTotals.window(t), 0...23)
        XCTAssertEqual(HourlyTotals.window([Int64](repeating: 0, count: 24)), 9...17)
    }
    func testHourLabels() { XCTAssertEqual(HourlyTotals.label(0), "12a"); XCTAssertEqual(HourlyTotals.label(9), "9a"); XCTAssertEqual(HourlyTotals.label(12), "12p"); XCTAssertEqual(HourlyTotals.label(17), "5p") }
    func testReceiptSaysOnlyWhatIsKnown() {
        let r = ReceiptText.make(merchant: "Chai Point", amount: Money(minor: 12_550), status: "succeeded", id: "pi_abc", when: nil, refundedMinor: 2_000)
        XCTAssertTrue(r.contains("Receipt from Chai Point")); XCTAssertTrue(r.contains("Paid")); XCTAssertTrue(r.contains("Refunded")); XCTAssertTrue(r.contains("pi_abc")); XCTAssertTrue(r.contains("no real money"))
        XCTAssertFalse(ReceiptText.make(merchant: "", amount: Money(minor: 1), status: "failed", id: "x", when: nil).contains("Refunded"))
    }
}
