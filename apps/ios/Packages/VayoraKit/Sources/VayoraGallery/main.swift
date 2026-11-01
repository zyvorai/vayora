import AppKit
import SwiftUI
import VayoraKit
import VayoraUI

// Renders screens to PNG so the design can be reviewed without a simulator.
// usage: VayoraGallery <output-dir>
let out = CommandLine.arguments.dropFirst().first ?? "gallery"
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)

@MainActor func render<V: View>(_ name: String, _ view: V, size: CGSize = CGSize(width: 393, height: 852)) {
    let r = ImageRenderer(content: view.frame(width: size.width, height: size.height).environment(\.colorScheme, .dark))
    r.scale = 2
    guard let img = r.nsImage, let tiff = img.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else { print("FAILED \(name)"); return }
    try? png.write(to: URL(fileURLWithPath: "\(out)/\(name).png")); print("ok \(name)")
}

MainActor.assumeIsolated {
    let rows = [
        TxnRowModel(id: "1", name: "Zyvor Sandbox", payee: "sandbox@zyvor-sim", amount: Money(minor: 49900), when: "Today, 5:01 pm", ok: true),
        TxnRowModel(id: "2", name: "Chai Point", payee: "chai@upi", amount: Money(minor: 12000), when: "Yesterday", ok: true),
        TxnRowModel(id: "3", name: "Asha Stores", payee: "asha@okzyvor", amount: Money(minor: 250000), when: "Mon", ok: false),
    ]
    render("pay-home", PayHomeScreen(balance: Money(minor: 900_100), spentThisMonth: Money(minor: 143_390), week: [0, 120, 40, 0, 499, 200, 310], recent: rows, phase: 0.18, scrolls: false, summary: [(.food, 70_000), (.transfers, 50_000), (.bills, 18_400), (.other, 4_990)]), size: CGSize(width: 393, height: 1180))
    render("pay-review-0", PayReviewScreen(name: "Zyvor Sandbox", payee: "sandbox@zyvor-sim", amount: Money(minor: 49900), balance: Money(minor: 900_100), frozen: 0))
    render("pay-review-60", PayReviewScreen(name: "Zyvor Sandbox", payee: "sandbox@zyvor-sim", amount: Money(minor: 49900), balance: Money(minor: 900_100), frozen: 0.6))
    render("pay-home-empty", PayHomeScreen(balance: Money(minor: 1_000_000), spentThisMonth: Money(minor: 0), week: [0, 0, 0, 0, 0, 0, 0], recent: [], phase: 0.18, scrolls: false))
    render("pay-pin", PinScreen(filled: 2, onBiometric: {}))
    render("pay-pin-error", PinScreen(filled: 4, error: true, shake: 8))
    render("pay-paid-30", ResultScreen(kind: .paid, amount: Money(minor: 49900), to: "Zyvor Sandbox", progress: 0.3))
    render("pay-paid-70", ResultScreen(kind: .paid, amount: Money(minor: 49900), to: "Zyvor Sandbox", progress: 0.7))
    render("pay-paid-100", ResultScreen(kind: .paid, amount: Money(minor: 49900), to: "Zyvor Sandbox", progress: 1))
    render("pay-failed", ResultScreen(kind: .failed("Not enough test balance"), amount: Money(minor: 49900), to: "x", progress: 1))
    render("pay-checking", ResultScreen(kind: .checking, amount: Money(minor: 49900), to: "x", progress: 1))

    let mrows = [
        PaymentRowModel(id: "1", amount: Money(minor: 49900), when: "5:01 pm", note: "Table 4", state: .succeeded),
        PaymentRowModel(id: "2", amount: Money(minor: 12000), when: "4:40 pm", note: "Tea", state: .refunded),
        PaymentRowModel(id: "3", amount: Money(minor: 250000), when: "3:12 pm", state: .partRefunded),
        PaymentRowModel(id: "4", amount: Money(minor: 8000), when: "2:02 pm", state: .processing),
    ]
    render("m-keypad", CollectKeypadScreen(text: "499.5", note: "Table 4", chips: ["Tea", "Table 4", "Takeaway", "Tip"], canContinue: true))
    render("m-keypad-empty", CollectKeypadScreen(text: "", note: "", chips: ["Tea", "Table 4", "Takeaway", "Tip"], canContinue: false))
    let qr = QR.image("upi://pay?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am=499.00&cu=INR&tn=att_demo")
    render("m-qr", QRHeroScreen(amount: Money(minor: 49900), qr: qr, expiry: 0.72, pulse: 0.4, customerMode: false, note: "Table 4"))
    render("m-qr-customer", QRHeroScreen(amount: Money(minor: 49900), qr: qr, expiry: 0.72, pulse: 0.4, customerMode: true, note: "Table 4"))
    render("m-today", MerchantTodayScreen(collected: Money(minor: 49900), count: 4, average: Money(minor: 12475), trend: [0, 80, 120, 120, 499, 499, 499], rows: mrows, scrolls: false))
    render("m-paid", ResultScreen(kind: .paid, amount: Money(minor: 49900), to: "Table 4", progress: 0.8, brand: .merchant, paidTitle: "Received", doneLabel: "New payment"))

    // Navi-style screens
    let book = PayeeBook([Payee(vpa: "asha@okzyvor", name: "Asha Kulkarni", favourite: true, lastPaid: Date()), Payee(vpa: "9876543210@zyvor-test", name: "Ravi", lastPaid: Date().addingTimeInterval(-86_400)), Payee(vpa: "chai@upi", name: "Chai Point")])
    render("payees", PayeesScreen(payees: book.sorted, scrolls: false))
    let split = QR.image(RequestPayload.make(payee: "pay-7f3a@zyvor-test", name: "Susant", amount: Money(minor: 16_634), note: "Dinner"))
    render("split", SplitShareScreen(total: Money(minor: 49_900), people: 3, share: Money(minor: 16_634), qr: split, myName: "Susant"))
    render("reminders", RemindersScreen(rows: [
        ReminderRowModel(id: "1", name: "Rent", amount: Money(minor: 1_200_000), schedule: "5th of each month", next: "5 Nov, 9:00 am", enabled: true),
        ReminderRowModel(id: "2", name: "Electricity", amount: Money(minor: 184_000), schedule: "12th of each month", next: "12 Nov, 9:00 am", enabled: true),
        ReminderRowModel(id: "3", name: "Broadband", amount: Money(minor: 79_900), schedule: "20th of each month", next: "", enabled: false)], scrolls: false))
    render("month-card", ZStack { AmbientBackground(brand: .pay, phase: 0.2); MonthSummaryCard(items: [(.food, 70_000), (.transfers, 50_000), (.bills, 18_400), (.other, 4_990)], total: Money(minor: 143_390)).padding(20) }, size: CGSize(width: 393, height: 330))

    // AutoPay
    let deb = [DebitRowModel(id: "1", amount: Money(minor: 250_000), when: "1 Nov", state: "captured"),
               DebitRowModel(id: "2", amount: Money(minor: 250_000), when: "1 Dec", state: "notified"),
               DebitRowModel(id: "3", amount: Money(minor: 250_013), when: "1 Jan", state: "failed")]
    render("ap-review-0", MandateReviewScreen(merchant: "Zyvor Sandbox", payee: "sandbox@zyvor-sim", cap: Money(minor: 500_000), frequency: "Every month", until: "1 Nov 2027", frozen: 0))
    render("ap-review-60", MandateReviewScreen(merchant: "Zyvor Sandbox", payee: "sandbox@zyvor-sim", cap: Money(minor: 500_000), frequency: "Every month", until: "1 Nov 2027", frozen: 0.6))
    let cards = [PayerMandateCardModel(id: "mnd_1", merchant: "Zyvor Sandbox", payee: "sandbox@zyvor-sim", cap: Money(minor: 500_000), frequency: "Every month", until: "1 Nov 2027", status: .active, taken: Money(minor: 250_000), recent: [DebitRowModel(id: "a", amount: Money(minor: 250_000), when: "1 Nov, 10:02 am", state: "captured")]),
                 PayerMandateCardModel(id: "mnd_2", merchant: "Chai Point", payee: "chai@upi", cap: Money(minor: 150_000), frequency: "Every week", until: nil, status: .revoked(by: "payer"), taken: Money(minor: 90_000), recent: [])]
    render("ap-pay-list", ZStack(alignment: .top) { AmbientBackground(brand: .pay, phase: 0.4); VStack(alignment: .leading, spacing: 14) { HStack { Text("AutoPay").vfont(28, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }; PayerAutoPayList(cards: cards) }.padding(20) }, size: CGSize(width: 393, height: 980))
    render("ap-pay-empty", ZStack(alignment: .top) { AmbientBackground(brand: .pay, phase: 0.4); VStack(alignment: .leading, spacing: 14) { HStack { Text("AutoPay").vfont(28, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }; PayerAutoPayList(cards: []) }.padding(20) }, size: CGSize(width: 393, height: 520))
    let apRows = [MandateRowModel(id: "mnd_a", name: "Ravi Kumar", line: "Up to ₹5,000.00 · Every month", status: .active),
                MandateRowModel(id: "mnd_b", name: "Asha Rao", line: "Up to ₹1,500.00 · Every week", status: .waiting),
                MandateRowModel(id: "mnd_c", name: "Chai Point", line: "Up to ₹800.00 · Every month", status: .revoked(by: "payer"))]
    render("ap-m-list", AutoPayListScreen(rows: apRows, scrolls: false))
    let mqr = QR.image("upi://mandate?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am=5000.00&cu=INR&tn=mnd_demo&recur=monthly&start=2026-11-01&end=2027-11-01")
    render("ap-m-qr", MandateQRScreen(customer: "Ravi", cap: Money(minor: 500_000), frequency: "Every month", until: "1 Nov 2027", qr: mqr, status: .waiting))
    render("ap-m-qr-active", MandateQRScreen(customer: "Ravi", cap: Money(minor: 500_000), frequency: "Every month", until: "1 Nov 2027", qr: mqr, status: .active, approved: 1))
    render("ap-m-detail", MandateDetailScreen(name: "Ravi Kumar", cap: Money(minor: 500_000), line: "Every month · until 1 Nov 2027", status: .active, debits: deb, scrolls: false))
    render("ap-m-new", NewAutoPayDetailsScreen(name: .constant("Ravi Kumar"), frequency: .constant(.monthly), months: .constant(12), canContinue: true))
    render("m-today-hourly", MerchantTodayScreen(collected: Money(minor: 49900), count: 4, average: Money(minor: 12475), trend: [0, 80, 120, 120, 499, 499, 499], rows: mrows, scrolls: false,
                                                 hourly: HourlyBars.Model(hours: Array(9...15), minor: [0, 12_000, 18_500, 0, 9_400, 0, 10_000])), size: CGSize(width: 393, height: 1000))
}
