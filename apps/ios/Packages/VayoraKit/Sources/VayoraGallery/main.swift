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
    render("pay-home", PayHomeScreen(balance: Money(minor: 900_100), spentThisMonth: Money(minor: 99_800), week: [0, 120, 40, 0, 499, 200, 310], recent: rows, phase: 0.18, scrolls: false))
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
}
