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

final class PaymentRiskTests: XCTestCase {
    func row(_ vpa: String, _ minor: Int64, paid: Bool = true, at t: TimeInterval = 1_700_000_000, kind: String = "") -> HistoryRow {
        HistoryRow(date: Date(timeIntervalSince1970: t), name: vpa, payee: vpa, amountMinor: minor, paid: paid, category: .other, note: "", kind: kind)
    }
    func testFirstTimeOnlyWhenNeverPaidAndNotSaved() {
        XCTAssertEqual(PaymentRisk.check(payee: "new@bank", amountMinor: 100, history: [], saved: []), [.firstTime])
        XCTAssertEqual(PaymentRisk.check(payee: "new@bank", amountMinor: 100, history: [], saved: [Payee(vpa: "new@bank", name: "N")]), [])
        XCTAssertEqual(PaymentRisk.check(payee: "new@bank", amountMinor: 100, history: [row("new@bank", 5)], saved: []), [])
        XCTAssertEqual(PaymentRisk.check(payee: "new@bank", amountMinor: 100, history: [row("new@bank", 5, paid: false)], saved: []), [.firstTime])
    }
    func testLookalikeIsOneEditFromASavedPayee() {
        let saved = [Payee(vpa: "asha@okzyvor", name: "Asha")]
        XCTAssertEqual(PaymentRisk.check(payee: "ashaa@okzyvor", amountMinor: 1, history: [], saved: saved), [.firstTime, .lookalike(of: "asha@okzyvor")])
        XCTAssertFalse(PaymentRisk.check(payee: "ravi@okzyvor", amountMinor: 1, history: [], saved: saved).contains(.lookalike(of: "asha@okzyvor")))
        XCTAssertEqual(PaymentRisk.check(payee: "asha@okzyvor", amountMinor: 1, history: [], saved: saved), [])   // the saved payee itself
    }
    func testUnusuallyLargeNeedsHistoryAndFiveTimesTheMedian() {
        let h = [row("a@b.c", 100_00), row("a@b.c", 120_00), row("a@b.c", 110_00)]
        let saved = [Payee(vpa: "a@b.c", name: "A")]
        XCTAssertEqual(PaymentRisk.check(payee: "a@b.c", amountMinor: 551_00, history: h, saved: saved), [.unusuallyLarge(typicalMinor: 110_00)])
        XCTAssertEqual(PaymentRisk.check(payee: "a@b.c", amountMinor: 550_00, history: h, saved: saved), [])
        XCTAssertEqual(PaymentRisk.check(payee: "a@b.c", amountMinor: 9_999_00, history: Array(h.prefix(2)), saved: saved), [])
    }
    func testCoolingOnlyForBigPaymentToSomeoneNew() {
        XCTAssertEqual(PaymentRisk.coolingSeconds([.firstTime], amountMinor: 5_000_00), 10)
        XCTAssertEqual(PaymentRisk.coolingSeconds([.firstTime], amountMinor: 4_999_99), 0)
        XCTAssertEqual(PaymentRisk.coolingSeconds([], amountMinor: 90_000_00), 0)
    }
    func testEditDistance() {
        XCTAssertEqual(PaymentRisk.editDistance("kitten", "sitting"), 3); XCTAssertEqual(PaymentRisk.editDistance("", "abc"), 3); XCTAssertEqual(PaymentRisk.editDistance("same", "same"), 0)
    }
    func testPayAgainNeedsTwoPaymentsAndPrefersNowish() {
        var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!
        func at(_ d: Int, _ h: Int) -> Date { c.date(from: DateComponents(year: 2026, month: 10, day: d, hour: h))! }
        func r(_ v: String, _ d: Date, paid: Bool = true) -> HistoryRow { HistoryRow(date: d, name: v, payee: v, amountMinor: 100, paid: paid, category: .food, note: "", kind: "") }
        let now = at(29, 9)   // a Thursday, 9 am
        let h = [r("chai@x", at(22, 9)), r("chai@x", at(15, 9)), r("gym@x", at(28, 20)), r("gym@x", at(27, 20)), r("once@x", at(29, 9)), r("bad@x", at(22, 9), paid: false), r("bad@x", at(15, 9), paid: false)]
        let s = PayAgain.suggest(h, now: now, calendar: c)
        XCTAssertEqual(s.map(\.vpa), ["chai@x", "gym@x"])
        XCTAssertEqual(s.first?.count, 2)
    }
    func testPayAgainIgnoresAutopayDebits() {
        let d = Date(); func r(_ k: String) -> HistoryRow { HistoryRow(date: d, name: "M", payee: "m@x", amountMinor: 1, paid: true, category: .bills, note: "", kind: k) }
        XCTAssertTrue(PayAgain.suggest([r("autopay"), r("autopay"), r("autopay")], now: d).isEmpty)
    }
}

final class SpendQueryTests: XCTestCase {
    var cal: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; c.firstWeekday = 2; return c }
    var now: Date { cal.date(from: DateComponents(year: 2026, month: 10, day: 29, hour: 15))! }   // Thursday
    func at(_ m: Int, _ d: Int, _ h: Int = 12) -> Date { cal.date(from: DateComponents(year: 2026, month: m, day: d, hour: h))! }
    func row(_ n: String, _ date: Date, _ minor: Int64, _ cat: SpendCategory, paid: Bool = true) -> HistoryRow { HistoryRow(date: date, name: n, payee: n.lowercased() + "@x", amountMinor: minor, paid: paid, category: cat, note: "", kind: "") }

    func testCategoryAndLastWeek() {
        let q = SpendQuery.parse("how much did I spend on food last week", now: now, calendar: cal)
        XCTAssertEqual(q.filter.category, .food); XCTAssertEqual(q.periodLabel, "last week"); XCTAssertTrue(q.isStructured); XCTAssertEqual(q.filter.text, "")
        let rows = [row("Chai", at(10, 21), 100_00, .food), row("Chai", at(10, 29), 50_00, .food), row("Metro", at(10, 22), 30_00, .travel), row("Chai", at(10, 14), 70_00, .food)]
        let hit = q.apply(rows, calendar: cal)
        XCTAssertEqual(hit.map(\.amountMinor), [100_00])
        XCTAssertEqual(SpendQuery.total(hit).minor, 100_00)
    }
    func testAmountRulesAreStrict() {
        let q = SpendQuery.parse("more than 500 this month", now: now, calendar: cal)
        XCTAssertEqual(q.minMinor, 500_00); XCTAssertEqual(q.periodLabel, "this month"); XCTAssertEqual(q.filter.text, "")
        let rows = [row("A", at(10, 3), 500_00, .other), row("B", at(10, 4), 500_01, .other), row("C", at(9, 30), 900_00, .other)]
        XCTAssertEqual(q.apply(rows, calendar: cal).map(\.name), ["B"])
        XCTAssertEqual(SpendQuery.parse("under 200", now: now, calendar: cal).maxMinor, 200_00)
    }
    func testPlainNameIsATextSearchNotStructured() {
        let q = SpendQuery.parse("Chai Point", now: now, calendar: cal)
        XCTAssertEqual(q.filter.text, "chai point"); XCTAssertFalse(q.isStructured)
    }
    func testLastNDaysAndYesterdayAndFailed() {
        XCTAssertEqual(SpendQuery.parse("last 7 days", now: now, calendar: cal).periodLabel, "last 7 days")
        let y = SpendQuery.parse("yesterday", now: now, calendar: cal)
        XCTAssertEqual(y.apply([row("A", at(10, 28), 1, .other), row("B", at(10, 29), 1, .other)], calendar: cal).map(\.name), ["A"])
        XCTAssertEqual(SpendQuery.parse("failed payments", now: now, calendar: cal).filter.outcome, .failed)
    }
    func testTotalCountsOnlyPaid() {
        let t = SpendQuery.total([row("A", now, 100, .food), row("B", now, 900, .food, paid: false)])
        XCTAssertEqual(t.minor, 100); XCTAssertEqual(t.count, 1)
    }
    func testEmptyAndNonsenseDoNotCrash() {
        XCTAssertFalse(SpendQuery.parse("", now: now, calendar: cal).isStructured)
        _ = SpendQuery.parse("more than", now: now, calendar: cal); _ = SpendQuery.parse("last 99999 days", now: now, calendar: cal)
    }
}

final class SoundboxPhraseTests: XCTestCase {
    func testEnglish() {
        XCTAssertEqual(SoundboxPhrase.received(Money(minor: 250_00), language: .english), "250 rupees received")
        XCTAssertEqual(SoundboxPhrase.received(Money(minor: 1_00), language: .english), "1 rupee received")
        XCTAssertEqual(SoundboxPhrase.received(Money(minor: 12_50), language: .english), "12 rupees and 50 paise received")
        XCTAssertEqual(SoundboxPhrase.received(Money(minor: 50), language: .english), "0 rupees and 50 paise received")
    }
    func testHindi() {
        XCTAssertEqual(SoundboxPhrase.received(Money(minor: 250_00), language: .hindi), "250 रुपये प्राप्त हुए")
        XCTAssertEqual(SoundboxPhrase.received(Money(minor: 12_05), language: .hindi), "12 रुपये 5 पैसे प्राप्त हुए")
    }
    func testVoiceTags() { XCTAssertEqual(SoundboxPhrase.Language.hindi.rawValue, "hi-IN"); XCTAssertEqual(SoundboxPhrase.Language.english.rawValue, "en-IN") }
}

final class ClosingDecodeTests: XCTestCase {
    func testClosingDecodesAndSaysShortOrOver() throws {
        let j = #"{"id":"clo_1","expected_cash_minor":150000,"counted_cash_minor":149500,"difference_minor":-500,"digital":[{"method":"upi","payments":3,"gross_minor":9000,"refunded_minor":0,"net_minor":9000}],"status":"pending_signoff"}"#
        let c = try JSONDecoder().decode(ClosingInfo.self, from: Data(j.utf8))
        XCTAssertEqual(c.verdict, "Short"); XCTAssertEqual(c.digital.first?.netMinor, 9000); XCTAssertEqual(c.status, "pending_signoff")
    }
    func testVerdictWords() {
        func v(_ d: Int64) -> String { ClosingInfo(id: "x", expectedCashMinor: 0, countedCashMinor: 0, differenceMinor: d, digital: [], status: "").verdict }
        XCTAssertEqual(v(0), "Matches"); XCTAssertEqual(v(5), "Over"); XCTAssertEqual(v(-5), "Short")
    }
}

final class BranchDecodeTests: XCTestCase {
    func testBranchDecodes() throws {
        let b = try JSONDecoder().decode(BranchInfo.self, from: Data(#"{"id":"br_1","name":"Main stall"}"#.utf8))
        XCTAssertEqual(b.name, "Main stall")
    }
}

final class PlannedTests: XCTestCase {
    var cal: Calendar { var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!; return c }
    func d(_ m: Int, _ day: Int, _ h: Int = 10) -> Date { cal.date(from: DateComponents(year: 2026, month: m, day: day, hour: h))! }
    func r(_ v: String, _ date: Date, _ minor: Int64 = 199_00, paid: Bool = true, kind: String = "") -> HistoryRow { HistoryRow(date: date, name: v, payee: v, amountMinor: minor, paid: paid, category: .other, note: "", kind: kind) }

    func testMonthlyIsFoundAndNextIsPredicted() {
        let f = RecurringDetector.find([r("ott@x", d(8, 5)), r("ott@x", d(9, 4)), r("ott@x", d(10, 5))], calendar: cal)
        XCTAssertEqual(f.count, 1); XCTAssertEqual(f[0].cadence, .monthly); XCTAssertEqual(f[0].count, 3)
        XCTAssertEqual(cal.dateComponents([.month, .day], from: f[0].next).month, 11)
    }
    func testWeeklyIsFound() {
        let f = RecurringDetector.find([r("milk@x", d(10, 1)), r("milk@x", d(10, 8)), r("milk@x", d(10, 15)), r("milk@x", d(10, 22))], calendar: cal)
        XCTAssertEqual(f.first?.cadence, .weekly)
    }
    func testNotFoundWhenIrregularOrTooFewOrAmountsVaryOrAutopayOrFailed() {
        XCTAssertTrue(RecurringDetector.find([r("a@x", d(10, 1)), r("a@x", d(10, 20))], calendar: cal).isEmpty)                                    // two
        XCTAssertTrue(RecurringDetector.find([r("a@x", d(8, 1)), r("a@x", d(8, 20)), r("a@x", d(10, 20))], calendar: cal).isEmpty)                 // irregular
        XCTAssertTrue(RecurringDetector.find([r("a@x", d(8, 5), 100_00), r("a@x", d(9, 4), 500_00), r("a@x", d(10, 5), 100_00)], calendar: cal).isEmpty) // amounts
        XCTAssertTrue(RecurringDetector.find([r("a@x", d(8, 5), kind: "autopay"), r("a@x", d(9, 4), kind: "autopay"), r("a@x", d(10, 5), kind: "autopay")], calendar: cal).isEmpty)
        XCTAssertTrue(RecurringDetector.find([r("a@x", d(8, 5), paid: false), r("a@x", d(9, 4), paid: false), r("a@x", d(10, 5), paid: false)], calendar: cal).isEmpty)
    }
    func testSmallAmountDriftIsTolerated() {
        XCTAssertEqual(RecurringDetector.find([r("e@x", d(8, 5), 100_00), r("e@x", d(9, 5), 115_00), r("e@x", d(10, 5), 90_00)], calendar: cal).count, 1)
    }
    func testScheduleBookDueAllDayAndNotDone() {
        var a = ScheduledPayment(vpa: "a@x", name: "A", amountMinor: 1, date: d(11, 1, 23)), b = ScheduledPayment(vpa: "b@x", name: "B", amountMinor: 1, date: d(11, 2, 9)), c = ScheduledPayment(vpa: "c@x", name: "C", amountMinor: 1, date: d(10, 30))
        c.done = true; a.note = "x"; b.note = "y"
        let now = d(11, 1, 8)
        XCTAssertEqual(ScheduleBook.due([a, b, c], now: now, calendar: cal).map(\.name), ["A"])
        XCTAssertEqual(ScheduleBook.upcoming([b, a, c]).map(\.name), ["A", "B"])
    }
    func testSplitRecord() {
        var s = SplitRecord(date: Date(), note: "Dinner", totalMinor: 900_00, shareMinor: 300_00, people: 3)
        XCTAssertEqual(s.members.count, 2); XCTAssertEqual(s.owedMinor, 600_00); XCTAssertFalse(s.isSettled)
        s.members[0].paid = true; XCTAssertEqual(s.owedMinor, 300_00)
        s.members[1].paid = true; XCTAssertTrue(s.isSettled); XCTAssertEqual(s.owedMinor, 0)
        XCTAssertEqual(SplitRecord(date: Date(), note: "", totalMinor: 1, shareMinor: 1, people: 1).members.count, 1)   // never zero friends
    }
    func testCodableRoundTrip() throws {
        let p = ScheduledPayment(vpa: "a@x", name: "A", amountMinor: 5, date: Date(timeIntervalSince1970: 1_700_000_000), note: "n")
        XCTAssertEqual(try JSONDecoder().decode(ScheduledPayment.self, from: JSONEncoder().encode(p)), p)
    }
}

final class CashierGateTests: XCTestCase {
    let t0 = Date(timeIntervalSince1970: 1_700_000_000)
    func testPINShape() {
        XCTAssertTrue(PinGate.isValidPIN("1234")); XCTAssertTrue(PinGate.isValidPIN("123456"))
        XCTAssertFalse(PinGate.isValidPIN("123")); XCTAssertFalse(PinGate.isValidPIN("1234567")); XCTAssertFalse(PinGate.isValidPIN("12a4")); XCTAssertFalse(PinGate.isValidPIN("١٢٣٤"))
    }
    func testRightPINOpensAndResetsFailures() {
        var g = PinGate()
        XCTAssertFalse(g.attempt("0000", against: "1234", now: t0)); XCTAssertEqual(g.failures, 1)
        XCTAssertTrue(g.attempt("1234", against: "1234", now: t0)); XCTAssertEqual(g.failures, 0)
    }
    func testFifthWrongLocksForThirtySecondsEvenForTheRightPIN() {
        var g = PinGate()
        for _ in 0..<5 { XCTAssertFalse(g.attempt("9999", against: "1234", now: t0)) }
        XCTAssertTrue(g.isLocked(now: t0)); XCTAssertEqual(g.secondsLeft(now: t0), 30)
        XCTAssertFalse(g.attempt("1234", against: "1234", now: t0.addingTimeInterval(10)))
        XCTAssertFalse(g.isLocked(now: t0.addingTimeInterval(30)))
        XCTAssertTrue(g.attempt("1234", against: "1234", now: t0.addingTimeInterval(31)))
    }
    func testAfterLockoutFailuresStartAgain() {
        var g = PinGate()
        for _ in 0..<5 { _ = g.attempt("x", against: "1234", now: t0) }
        XCTAssertFalse(g.attempt("0000", against: "1234", now: t0.addingTimeInterval(40))); XCTAssertEqual(g.failures, 1)
    }
}

final class RegularsTests: XCTestCase {
    func testTotalsPerNameIgnoreCaseAndUnpaid() {
        var b = RegularsBook()
        b.tag("p1", as: " Ravi  Kumar "); b.tag("p2", as: "ravi kumar"); b.tag("p3", as: "Asha"); b.tag("p4", as: "Ravi Kumar")
        let t = b.totals([("p1", 100, true), ("p2", 50, true), ("p3", 500, true), ("p4", 900, false), ("p9", 7, true)])
        XCTAssertEqual(t.map(\.name), ["Asha", "Ravi Kumar"]); XCTAssertEqual(t[1].minor, 150); XCTAssertEqual(t[1].count, 2)
    }
    func testEmptyNameUntagsAndLongNameIsCut() {
        var b = RegularsBook(); b.tag("p1", as: "Ravi"); b.tag("p1", as: "   "); XCTAssertNil(b.name(for: "p1"))
        b.tag("p2", as: String(repeating: "a", count: 100)); XCTAssertEqual(b.name(for: "p2")?.count, 40)
    }
    func testNamesMostUsedFirstAndCodable() throws {
        var b = RegularsBook(); b.tag("1", as: "Asha"); b.tag("2", as: "Ravi"); b.tag("3", as: "ravi")
        XCTAssertEqual(b.names.first?.lowercased(), "ravi")
        XCTAssertEqual(try JSONDecoder().decode(RegularsBook.self, from: JSONEncoder().encode(b)), b)
    }
}

final class CategoryMemoryTests: XCTestCase {
    func testRememberedBeatsGuessAndOthersStillGuess() {
        var m = CategoryMemory()
        XCTAssertEqual(m.resolve(vpa: "cafe@x", name: "Cafe Blue", note: "", transfer: false), .food)   // the guess
        m.set("CAFE@x", to: .shopping)
        XCTAssertEqual(m.resolve(vpa: "cafe@x", name: "Cafe Blue", note: "", transfer: false), .shopping)
        XCTAssertEqual(m.resolve(vpa: "other@x", name: "Metro", note: "", transfer: false), .travel)
        m.forget("cafe@x"); XCTAssertEqual(m.resolve(vpa: "cafe@x", name: "Cafe Blue", note: "", transfer: false), .food)
    }
    func testAnOverrideAlsoWinsForTransfersAndRoundTrips() throws {
        var m = CategoryMemory(); m.set("friend@x", to: .bills)
        XCTAssertEqual(m.resolve(vpa: "friend@x", name: "Friend", note: "", transfer: true), .bills)
        XCTAssertEqual(try JSONDecoder().decode(CategoryMemory.self, from: JSONEncoder().encode(m)), m)
    }
    func testUnknownStoredValueIsIgnored() throws {
        let m = try JSONDecoder().decode(CategoryMemory.self, from: Data(#"{"byPayee":{"a@x":"nonsense"}}"#.utf8))
        XCTAssertNil(m.category(for: "a@x"))
    }
}

final class JarTests: XCTestCase {
    func testMoneyIsConservedThroughDepositWithdrawRemove() throws {
        var b = JarBook(); var wallet: Int64 = 10_000_00
        let j = try b.add(name: " Trip ", targetMinor: 5_000_00)
        XCTAssertEqual(j.name, "Trip")
        try b.deposit(j.id, 3_000_00, wallet: &wallet)
        XCTAssertEqual(wallet, 7_000_00); XCTAssertEqual(b.totalSavedMinor, 3_000_00); XCTAssertEqual(wallet + b.totalSavedMinor, 10_000_00)
        try b.withdraw(j.id, 1_000_00, wallet: &wallet); XCTAssertEqual(wallet + b.totalSavedMinor, 10_000_00)
        b.remove(j.id, wallet: &wallet); XCTAssertEqual(wallet, 10_000_00); XCTAssertTrue(b.jars.isEmpty)
    }
    func testBoundsNeverGoBelowZero() throws {
        var b = JarBook(); var wallet: Int64 = 100_00
        let j = try b.add(name: "A", targetMinor: 50_00)
        XCTAssertThrowsError(try b.deposit(j.id, 100_01, wallet: &wallet)) { XCTAssertEqual($0 as? JarBook.Failure, .walletTooLow) }
        XCTAssertThrowsError(try b.withdraw(j.id, 1, wallet: &wallet)) { XCTAssertEqual($0 as? JarBook.Failure, .jarTooLow) }
        XCTAssertThrowsError(try b.deposit(j.id, 0, wallet: &wallet)) { XCTAssertEqual($0 as? JarBook.Failure, .badAmount) }
        XCTAssertThrowsError(try b.deposit(UUID(), 1, wallet: &wallet)) { XCTAssertEqual($0 as? JarBook.Failure, .unknownJar) }
        XCTAssertEqual(wallet, 100_00); XCTAssertEqual(b.totalSavedMinor, 0)
    }
    func testValidationAndLimit() throws {
        var b = JarBook()
        XCTAssertThrowsError(try b.add(name: "  ", targetMinor: 1)) { XCTAssertEqual($0 as? JarBook.Failure, .badName) }
        XCTAssertThrowsError(try b.add(name: "A", targetMinor: 0)) { XCTAssertEqual($0 as? JarBook.Failure, .badTarget) }
        XCTAssertThrowsError(try b.add(name: String(repeating: "x", count: 31), targetMinor: 1)) { XCTAssertEqual($0 as? JarBook.Failure, .badName) }
        for i in 0..<JarBook.maxJars { try b.add(name: "J\(i)", targetMinor: 1) }
        XCTAssertThrowsError(try b.add(name: "More", targetMinor: 1)) { XCTAssertEqual($0 as? JarBook.Failure, .tooMany) }
    }
    func testProgress() throws {
        var b = JarBook(); var w: Int64 = 1000_00
        let j = try b.add(name: "A", targetMinor: 200_00)
        try b.deposit(j.id, 100_00, wallet: &w); XCTAssertEqual(b.jars[0].fraction, 0.5, accuracy: 0.001); XCTAssertFalse(b.jars[0].reached)
        try b.deposit(j.id, 150_00, wallet: &w); XCTAssertEqual(b.jars[0].fraction, 1); XCTAssertTrue(b.jars[0].reached)
    }
}

final class VaultTests: XCTestCase {
    func testTagParsing() {
        XCTAssertEqual(Vault.parseTags("Trip, office lunch #tax;TRIP"), ["trip", "office", "lunch", "tax"])
        XCTAssertEqual(Vault.parseTags("###, ,"), [])
        XCTAssertEqual(Vault.parseTags((1...12).map { "t\($0)" }.joined(separator: " ")).count, Vault.maxTags)
        XCTAssertEqual(Vault.parseTags(String(repeating: "a", count: 50)).first?.count, Vault.maxTagLength)
    }
    func testEntriesAreRemovedWhenEmptyAndNoteIsCapped() {
        var v = Vault(); v.setNote("p1", "  Birthday gift "); v.setTags("p1", "gift"); XCTAssertEqual(v.entry("p1").note, "Birthday gift"); XCTAssertEqual(v.entry("p1").tags, ["gift"])
        v.setNote("p1", ""); v.setTags("p1", ""); XCTAssertTrue(v.entries.isEmpty)
        v.setNote("p2", String(repeating: "x", count: 500)); XCTAssertEqual(v.entry("p2").note.count, Vault.maxNoteLength)
    }
    func testPhotoNameCannotClimbOutOfTheFolder() {
        var v = Vault()
        for bad in ["../x.jpg", "/etc/passwd", "a/b.jpg", ".hidden", "a..jpg", "x.tar.gz", "", String(repeating: "a", count: 90), "naïve.jpg"] { v.setPhoto("p", bad); XCTAssertNil(v.entry("p").photo, bad) }
        v.setPhoto("p", "3F2A-9b.jpg"); XCTAssertEqual(v.entry("p").photo, "3F2A-9b.jpg")
    }
    func testTagsAreSearchable() {
        let r = HistoryRow(date: Date(), name: "Shop", payee: "s@x", amountMinor: 1, paid: true, category: .shopping, note: "", kind: "", tags: ["birthday"])
        var f = HistoryFilter(); f.text = "birthday"; XCTAssertEqual(f.apply([r]).count, 1)
        f.text = "wedding"; XCTAssertEqual(f.apply([r]).count, 0)
    }
    func testMonthReportOnlyPaidInThatMonth() {
        var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!
        func d(_ m: Int, _ day: Int) -> Date { c.date(from: DateComponents(year: 2026, month: m, day: day, hour: 12))! }
        func r(_ date: Date, _ minor: Int64, _ cat: SpendCategory, paid: Bool = true) -> HistoryRow { HistoryRow(date: date, name: "n", payee: "p@x", amountMinor: minor, paid: paid, category: cat, note: "", kind: "") }
        let rep = MonthReport.make([r(d(10, 20), 100, .food), r(d(10, 2), 300, .travel), r(d(9, 30), 999, .food), r(d(10, 5), 500, .food, paid: false)], month: d(10, 15), calendar: c)
        XCTAssertEqual(rep.title, "October 2026"); XCTAssertEqual(rep.totalMinor, 400); XCTAssertEqual(rep.lines.map(\.amountMinor), [300, 100]); XCTAssertEqual(rep.byCategory.first?.category, .travel)
    }
}

final class ItemSplitTests: XCTestCase {
    func testSharesAlwaysAddUpToTheTotal() {
        let items = [ItemSplit.Item(name: "Pizza", amountMinor: 100_01, people: [0, 1, 2]), ItemSplit.Item(name: "Wine", amountMinor: 55_55, people: [1]), ItemSplit.Item(name: "Tip", amountMinor: 10_00, people: [])]
        let s = ItemSplit.shares(items, peopleCount: 3)
        XCTAssertEqual(s.reduce(0, +), 100_01 + 55_55 + 10_00)
        // pizza 100.01 / 3 = 33.33 r2 (people 0 and 1 get the extra paisa); tip 10.00 / 3 = 3.33 r1 (person 0 gets it)
        XCTAssertEqual(s[1], 33_34 + 55_55 + 3_33)
    }
    func testRemainderPaiseGoToTheFirstPeople() {
        XCTAssertEqual(ItemSplit.shares([.init(name: "x", amountMinor: 10, people: [0, 1, 2])], peopleCount: 3), [4, 3, 3])
    }
    func testBadAssignmentsAreIgnoredSafely() {
        XCTAssertEqual(ItemSplit.shares([.init(name: "x", amountMinor: 100, people: [7])], peopleCount: 2), [0, 0])
        XCTAssertEqual(ItemSplit.shares([.init(name: "x", amountMinor: -5, people: [0])], peopleCount: 1), [0])
        XCTAssertTrue(ItemSplit.shares([], peopleCount: 0).isEmpty)
    }
}

final class ItemisedRecordTests: XCTestCase {
    func testOwedUsesEachPersonsOwnShare() {
        var r = SplitRecord(date: Date(), note: "x", totalMinor: 1000, owing: [("Asha", 300), ("Ravi", 450)])
        XCTAssertEqual(r.owedMinor, 750); r.members[0].paid = true; XCTAssertEqual(r.owedMinor, 450); r.members[1].paid = true; XCTAssertTrue(r.isSettled)
    }
    func testOldRecordsWithoutPerMemberSharesStillDecode() throws {
        let old = try JSONEncoder().encode(SplitRecord(date: Date(), note: "x", totalMinor: 900, shareMinor: 300, people: 3))
        let r = try JSONDecoder().decode(SplitRecord.self, from: old)
        XCTAssertEqual(r.owedMinor, 600)
    }
}

final class PaymentsCSVTests: XCTestCase {
    var c: Calendar { PaymentsCSV.ist }
    func at(_ d: Int, _ h: Int, _ m: Int = 0) -> Date { c.date(from: DateComponents(year: 2026, month: 10, day: d, hour: h, minute: m))! }
    func test_IST_day_even_near_midnight() {
        // 23:50 IST on the 30th is 18:20 UTC the same day; 00:10 IST on the 31st is 18:40 UTC on the 30th
        let rows = [PaymentCSVRow(date: at(30, 23, 50), id: "a", amountMinor: 100_00, status: "succeeded", refundedMinor: 0), PaymentCSVRow(date: at(31, 0, 10), id: "b", amountMinor: 50_00, status: "succeeded", refundedMinor: 0)]
        let d = PaymentsCSV.daily(rows)
        XCTAssertTrue(d.contains("2026-10-30,1,100.00,0.00,100.00")); XCTAssertTrue(d.contains("2026-10-31,1,50.00,0.00,50.00"))
    }
    func testDailyNetsRefundsAndSkipsUnpaid() {
        let rows = [PaymentCSVRow(date: at(30, 10), id: "a", amountMinor: 300_00, status: "succeeded", refundedMinor: 100_00), PaymentCSVRow(date: at(30, 11), id: "b", amountMinor: 900_00, status: "failed", refundedMinor: 0),
                    PaymentCSVRow(date: at(30, 12), id: "c", amountMinor: 50_00, status: "succeeded", refundedMinor: 999_00)]
        XCTAssertTrue(PaymentsCSV.daily(rows).contains("2026-10-30,2,350.00,150.00,200.00"))   // an over-refund is capped at the payment
    }
    func testPaymentsFileShapeAndFormulaGuard() {
        let r = PaymentCSVRow(date: at(30, 9, 5), id: "=cmd|' /C calc'!A0", amountMinor: 12_50, status: "succeeded", refundedMinor: 2_50)
        let csv = PaymentsCSV.payments([r])
        XCTAssertTrue(csv.hasPrefix("date_ist,time_ist,payment_id,amount_inr,status,refunded_inr,net_inr\n"))
        XCTAssertTrue(csv.contains("2026-10-30,09:05,'=cmd|' /C calc'!A0,12.50,succeeded,2.50,10.00"))
        XCTAssertEqual(csv.split(separator: "\n").count, 2)
    }
    func testOldestFirst() {
        let csv = PaymentsCSV.payments([PaymentCSVRow(date: at(31, 9), id: "late", amountMinor: 1, status: "succeeded", refundedMinor: 0), PaymentCSVRow(date: at(30, 9), id: "early", amountMinor: 1, status: "succeeded", refundedMinor: 0)])
        XCTAssertLessThan(csv.range(of: "early")!.lowerBound, csv.range(of: "late")!.lowerBound)
    }
}

final class PrivacyTests: XCTestCase {
    let now = Date(timeIntervalSince1970: 1_700_000_000)
    func testLockPolicy() {
        XCTAssertFalse(LockPolicy.shouldLock(enabled: false, backgroundedAt: nil, now: now, timeoutMinutes: 0))
        XCTAssertTrue(LockPolicy.shouldLock(enabled: true, backgroundedAt: nil, now: now, timeoutMinutes: 5))               // launch
        XCTAssertTrue(LockPolicy.shouldLock(enabled: true, backgroundedAt: now, now: now, timeoutMinutes: 0))               // immediately
        XCTAssertFalse(LockPolicy.shouldLock(enabled: true, backgroundedAt: now.addingTimeInterval(-299), now: now, timeoutMinutes: 5))
        XCTAssertTrue(LockPolicy.shouldLock(enabled: true, backgroundedAt: now.addingTimeInterval(-300), now: now, timeoutMinutes: 5))
        // a clock that moved backwards, or a negative timeout, never locks by itself
        XCTAssertFalse(LockPolicy.shouldLock(enabled: true, backgroundedAt: now.addingTimeInterval(60), now: now, timeoutMinutes: -3))
    }
    func testScamHints() {
        XCTAssertNotNil(ScamHints.check(name: "Paytm Cashback", note: ""))
        XCTAssertNotNil(ScamHints.check(name: "Shop", note: "Claim your REFUND"))
        XCTAssertNil(ScamHints.check(name: "Chai Point", note: "Table 4"))
    }
    func testBackupRoundTripKeepsOnlyKnownKeys() throws {
        let file = try Backup.make(["payees": [["vpa": "a@x"]], "myName": "Asha", "smallPayments": true, "balance": 12345, "testPIN": "7391", "serverURL": "http://x"])
        let text = String(decoding: file, as: UTF8.self)
        XCTAssertFalse(text.contains("testPIN")); XCTAssertFalse(text.contains("serverURL")); XCTAssertFalse(text.contains("7391"))
        let back = try Backup.read(file)
        XCTAssertEqual(Set(back.keys), ["payees", "myName", "smallPayments", "balance"]); XCTAssertEqual(back["myName"] as? String, "Asha")
    }
    func testReadRefusesWrongShapeVersionAppAndSize() throws {
        func f(_ d: [String: Any], version: Int = 1, app: String = "vayora-pay") throws -> Data { try JSONSerialization.data(withJSONObject: ["app": app, "version": version, "data": d]) }
        XCTAssertThrowsError(try Backup.read(f(["balance": -5]))) { XCTAssertEqual($0 as? Backup.Failure, .badValue("balance")) }
        XCTAssertThrowsError(try Backup.read(f(["smallPayments": 1]))) { XCTAssertEqual($0 as? Backup.Failure, .badValue("smallPayments")) }
        XCTAssertThrowsError(try Backup.read(f(["payees": "not a list"]))) { XCTAssertEqual($0 as? Backup.Failure, .badValue("payees")) }
        XCTAssertThrowsError(try Backup.read(f(["myName": String(repeating: "a", count: 200)])))
        XCTAssertThrowsError(try Backup.read(f([:], version: 2))) { XCTAssertEqual($0 as? Backup.Failure, .wrongVersion) }
        XCTAssertThrowsError(try Backup.read(f([:], app: "other"))) { XCTAssertEqual($0 as? Backup.Failure, .notBackup) }
        XCTAssertThrowsError(try Backup.read(Data("nope".utf8))) { XCTAssertEqual($0 as? Backup.Failure, .notBackup) }
        XCTAssertThrowsError(try Backup.read(Data(count: Backup.maxBytes + 1))) { XCTAssertEqual($0 as? Backup.Failure, .tooBig) }
    }
    func testReadDropsUnknownKeysSilently() throws {
        let back = try Backup.read(JSONSerialization.data(withJSONObject: ["app": "vayora-pay", "version": 1, "data": ["testPIN": "9999", "balance": 5]]))
        XCTAssertEqual(Set(back.keys), ["balance"])
    }
}

final class VoiceCommandTests: XCTestCase {
    let asha = Payee(vpa: "asha@x", name: "Asha"), ravi = Payee(vpa: "ravi@x", name: "Ravi Kumar"), ravi2 = Payee(vpa: "ravi2@x", name: "Ravi Shah")
    var all: [Payee] { [asha, ravi, ravi2] }
    func pay(_ s: String) -> VoiceCommand.Result { VoiceCommand.parse(s, payees: all) }

    func testEnglishForms() {
        XCTAssertEqual(pay("Pay Asha 200 rupees"), .pay(payee: asha, amount: Money(minor: 200_00)))
        XCTAssertEqual(pay("send asha ₹ 50.50"), .pay(payee: asha, amount: Money(minor: 50_50)))
        XCTAssertEqual(pay("please pay Asha 1,200 rs"), .pay(payee: asha, amount: Money(minor: 1200_00)))
    }
    func testHindiAndHinglish() {
        XCTAssertEqual(pay("Asha ko 300 rupaye bhejo"), .pay(payee: asha, amount: Money(minor: 300_00)))
        XCTAssertEqual(pay("Asha को ४५० रुपये भेजो"), .pay(payee: asha, amount: Money(minor: 450_00)))
    }
    func testAmbiguousAndUnknownAndMissingAmount() {
        XCTAssertEqual(pay("pay Ravi 100"), .ambiguous([ravi, ravi2], Money(minor: 100_00)))
        XCTAssertEqual(pay("pay Ravi Shah 100"), .pay(payee: ravi2, amount: Money(minor: 100_00)))
        XCTAssertEqual(pay("pay Meena 100"), .noPayee("meena"))
        XCTAssertEqual(pay("pay Asha"), .noAmount)
        XCTAssertEqual(pay("pay 100"), .noPayee(""))
    }
    func testNotAPaymentAndBadAmounts() {
        XCTAssertEqual(pay("what is my balance"), .notAPayment)
        XCTAssertEqual(pay("pay Asha 0"), .noAmount)
        XCTAssertEqual(pay("pay Asha 99999999999999 rupees"), .noAmount)
    }
    func testNeverInventsAPayee() {
        XCTAssertEqual(VoiceCommand.parse("pay Asha 200", payees: []), .noPayee("asha"))
    }
}

final class CircleTests: XCTestCase {
    func testLinkRoundTripAndRefusals() throws {
        let s = CirclePayload.make(id: "cir_abc123", primary: "Asha K", perPayment: Money(minor: 500_00), total: Money(minor: 2000_00))
        let l = try CirclePayload.parse(s)
        XCTAssertEqual(l.id, "cir_abc123"); XCTAssertEqual(l.primary, "Asha K"); XCTAssertEqual(l.perPayment.minor, 500_00); XCTAssertEqual(l.total.minor, 2000_00)
        XCTAssertThrowsError(try CirclePayload.parse("upi://pay?pa=a@b")) { XCTAssertEqual($0 as? CirclePayload.Failure, .notCircle) }
        XCTAssertThrowsError(try CirclePayload.parse("vayora://circle?id=../x&pn=A&per=1&total=2")) { XCTAssertEqual($0 as? CirclePayload.Failure, .badID) }
        XCTAssertThrowsError(try CirclePayload.parse("vayora://circle?id=cir_abc123&pn=&per=1&total=2")) { XCTAssertEqual($0 as? CirclePayload.Failure, .badName) }
        XCTAssertThrowsError(try CirclePayload.parse("vayora://circle?id=cir_abc123&pn=A&per=5&total=2")) { XCTAssertEqual($0 as? CirclePayload.Failure, .badAmount) }
        XCTAssertThrowsError(try CirclePayload.parse("vayora://circle?id=cir_abc123&id=cir_zzz999&pn=A&per=1&total=2")) { XCTAssertEqual($0 as? CirclePayload.Failure, .duplicateParameter) }
    }
    func testDelegateCoverage() {
        var r = CircleRecord(id: "cir_1", role: .delegate, other: "Asha", perPaymentMinor: 500_00, totalMinor: 800_00)
        XCTAssertTrue(r.canCover(500_00)); XCTAssertFalse(r.canCover(500_01)); XCTAssertFalse(r.canCover(0))
        r.remainingMinor = 300_00; XCTAssertFalse(r.canCover(400_00)); XCTAssertTrue(r.canCover(300_00))
        r.status = "revoked"; XCTAssertFalse(r.canCover(1))
        XCTAssertFalse(CircleRecord(id: "c", role: .primary, other: "x", perPaymentMinor: 1, totalMinor: 1).canCover(1))   // a primary does not pay through its own circle
    }
    func testCircleInfoDecodes() throws {
        let j = #"{"circle_id":"cir_1","primary":"Asha","delegate":"Ravi","status":"active","max_per_payment_minor":50000,"total_limit_minor":80000,"used_minor":40000,"remaining_minor":40000,"spends":[{"attempt_id":"att_1","amount_minor":40000,"at":"2026-11-01T10:00:00Z"}]}"#
        let c = try JSONDecoder().decode(CircleInfo.self, from: Data(j.utf8))
        XCTAssertEqual(c.remainingMinor, 40000); XCTAssertEqual(c.spends.first?.attemptID, "att_1")
    }
}

final class CatalogueTests: XCTestCase {
    func testValidation() throws {
        var c = Catalogue()
        XCTAssertThrowsError(try c.add(Product(name: "  ", priceMinor: 100))) { XCTAssertEqual($0 as? Catalogue.Failure, .emptyName) }
        XCTAssertThrowsError(try c.add(Product(name: "Tea", priceMinor: 0))) { XCTAssertEqual($0 as? Catalogue.Failure, .badPrice) }
        XCTAssertThrowsError(try c.add(Product(name: "Tea", priceMinor: 100, gstPercent: 101))) { XCTAssertEqual($0 as? Catalogue.Failure, .badGST) }
        XCTAssertThrowsError(try c.add(Product(name: String(repeating: "x", count: 41), priceMinor: 100))) { XCTAssertEqual($0 as? Catalogue.Failure, .nameTooLong) }
        try c.add(Product(name: "Tea", priceMinor: 1000))
        XCTAssertThrowsError(try c.add(Product(name: "tea ", priceMinor: 500))) { XCTAssertEqual($0 as? Catalogue.Failure, .duplicate) }
        XCTAssertEqual(c.products.count, 1)
    }
    func testUpdateKeepsOwnName() throws {
        var c = Catalogue(); let p = Product(name: "Tea", priceMinor: 1000); try c.add(p)
        var q = p; q.priceMinor = 1200; try c.update(q)
        XCTAssertEqual(c.products[0].priceMinor, 1200)
    }
    func testExclusiveGSTRoundsHalfUp() {
        // 5% of Rs 10.10 = 50.5 paise -> 51
        let l = BillLine(name: "x", quantity: 1, unitMinor: 1010, gstPercent: 5, gstInclusive: false)
        XCTAssertEqual(l.parts.gst, 51); XCTAssertEqual(l.parts.gross, 1061); XCTAssertEqual(l.parts.net, 1010)
        // 18% of Rs 100 = 1800 exactly
        let m = BillLine(name: "y", quantity: 2, unitMinor: 5000, gstPercent: 18, gstInclusive: false)
        XCTAssertEqual(m.parts.gst, 1800); XCTAssertEqual(m.parts.gross, 11800)
    }
    func testInclusiveGSTSplitsAndAddsBack() {
        // Rs 118 with 18% inside: net 100, gst 18
        let l = BillLine(name: "x", quantity: 1, unitMinor: 11800, gstPercent: 18, gstInclusive: true)
        XCTAssertEqual(l.parts.net, 10000); XCTAssertEqual(l.parts.gst, 1800); XCTAssertEqual(l.parts.gross, 11800)
        // an awkward one still adds back exactly
        let m = BillLine(name: "y", quantity: 3, unitMinor: 3333, gstPercent: 12, gstInclusive: true)
        XCTAssertEqual(m.parts.net + m.parts.gst, m.parts.gross); XCTAssertEqual(m.parts.gross, 9999)
    }
    func testNoGST() {
        let l = BillLine(name: "x", quantity: 4, unitMinor: 250, gstPercent: nil, gstInclusive: false)
        XCTAssertEqual(l.parts.gst, 0); XCTAssertEqual(l.parts.gross, 1000)
        XCTAssertEqual(BillLine(name: "x", quantity: 1, unitMinor: 250, gstPercent: 0, gstInclusive: true).parts.gross, 250)
    }
    func testBillTotalsAndLines() {
        var b = Bill(); let tea = Product(name: "Tea", priceMinor: 1000, gstPercent: 5), vada = Product(name: "Vada pav", priceMinor: 2000)
        b.add(tea); b.add(tea); b.add(vada)
        XCTAssertEqual(b.lines.count, 2); XCTAssertEqual(b.quantity(of: tea), 2)
        XCTAssertEqual(b.netMinor, 4000); XCTAssertEqual(b.gstMinor, 100); XCTAssertEqual(b.totalMinor, 4100)
        XCTAssertEqual(b.note(), "2 x Tea, 1 x Vada pav")
        b.remove(tea); b.remove(tea)
        XCTAssertEqual(b.quantity(of: tea), 0); XCTAssertEqual(b.totalMinor, 2000)
    }
    func testNoteIsCutWithMoreCount() {
        var b = Bill()
        for i in 0..<10 { b.add(Product(name: "Item number \(i)", priceMinor: 100)) }
        let n = b.note(limit: 50)
        XCTAssertLessThanOrEqual(n.count, 60); XCTAssertTrue(n.contains("more"))
    }
    func testQuantityIsCapped() {
        var b = Bill(); let p = Product(name: "Pen", priceMinor: 100)
        b.add(p, quantity: 5000); b.add(p, quantity: 5)
        XCTAssertEqual(b.quantity(of: p), Bill.maxQuantity)
    }
    func testSalesByItemCountsOnlyPaidAndRecent() {
        let t = Date(timeIntervalSince1970: 1_700_000_000)
        let tea = BillLine(name: "Tea", quantity: 2, unitMinor: 1000, gstPercent: nil, gstInclusive: false)
        let vada = BillLine(name: "Vada", quantity: 3, unitMinor: 2000, gstPercent: nil, gstInclusive: false)
        let log = [BillRecord(intentID: "a", date: t, lines: [tea, vada]), BillRecord(intentID: "b", date: t, lines: [tea]),
                   BillRecord(intentID: "old", date: t.addingTimeInterval(-86400 * 90), lines: [tea]), BillRecord(intentID: "unpaid", date: t, lines: [vada])]
        let rows = SalesByItem.top(log, paid: ["a", "b", "old"], since: t.addingTimeInterval(-86400 * 7))
        XCTAssertEqual(rows.map(\.name), ["Tea", "Vada"]) // 4 teas vs 3 vadas
        XCTAssertEqual(rows[0].quantity, 4); XCTAssertEqual(rows[0].grossMinor, 4000); XCTAssertEqual(rows[1].quantity, 3)
        XCTAssertTrue(SalesByItem.summary(rows, title: "Week").contains("Not a tax invoice"))
        XCTAssertTrue(SalesByItem.summary([], title: "Week").contains("No itemised sales"))
    }
}
