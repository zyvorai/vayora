import SwiftUI
import UIKit
import VayoraKit
import VayoraUI

/// Counter keypad, then the QR as the hero, then a felt "Received".
struct CollectView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var entry = AmountEntry()
    @State private var note = ""
    @State private var phase: Phase = .entering
    @State private var customerMode = false
    @State private var resultProgress = 0.0
    @State private var savedBrightness: CGFloat?

    enum Phase: Equatable {
        case entering, creating
        case waiting(intentId: String, attemptId: String, uri: String, amount: Money, started: Date)
        case paid(Money), failed(String), error(String)
    }

    static let chips = ["Takeaway", "Dine in", "Delivery", "Tip"]
    static let window: TimeInterval = 10 * 60   // how long the QR is shown as fresh

    var body: some View {
        Group {
            switch phase {
            case .entering, .creating:
                VStack(spacing: 0) {
                    CollectKeypadScreen(text: entry.text, note: note, chips: Self.chips, canContinue: entry.money != nil && phase == .entering && store.client != nil,
                                        actionLabel: phase == .creating ? "Creating…" : "Show QR",
                                        onDigit: { entry.digit($0) }, onPoint: { entry.point() }, onDelete: { entry.delete() },
                                        onChip: { note = (note == $0 ? "" : $0) }, onShowQR: start)
                    if store.client == nil { Text("Add your API key in Settings first, or scan a setup QR.").font(.footnote).foregroundStyle(VUI.textSecondary).padding(.bottom, 8).background(VUI.ink) }
                }
            case .waiting(_, let attempt, let uri, let amt, let started):
                TimelineView(.animation(minimumInterval: 1.0 / 30)) { ctx in
                    let elapsed = ctx.date.timeIntervalSince(started)
                    QRHeroScreen(amount: amt, qr: QR.image(uri), expiry: max(0, 1 - elapsed / Self.window),
                                 pulse: (ctx.date.timeIntervalSinceReferenceDate / 2.2).truncatingRemainder(dividingBy: 1),
                                 customerMode: customerMode, note: note, onCustomerMode: toggleCustomer, onCancel: { reset() })
                }
                .task(id: attempt) { await poll(attempt, amt) }
            case .paid(let m):
                ResultScreen(kind: .paid, amount: m, to: note.isEmpty ? "this payment" : note, progress: resultProgress, brand: .merchant, paidTitle: "Received", doneLabel: "New payment", onDone: reset)
            case .failed(let why):
                ResultScreen(kind: .failed(why), amount: Money(minor: 0), to: "", progress: 1, brand: .merchant, doneLabel: "New payment", onDone: reset)
            case .error(let why):
                ResultScreen(kind: .failed(why), amount: Money(minor: 0), to: "", progress: 1, brand: .merchant, doneLabel: "Back", onDone: reset)
            }
        }
        .background(VUI.ink)
        .animation(VUI.spring, value: phase)
        .onChange(of: phase) { _, new in
            if case .paid = new { resultProgress = 0; withAnimation(.easeOut(duration: 1.3)) { resultProgress = 1 } }
        }
        .onDisappear { endCustomerMode() }
    }

    func reset() {
        endCustomerMode(); entry.clear(); note = ""; phase = .entering
    }

    func toggleCustomer() { customerMode ? endCustomerMode() : beginCustomerMode() }
    func beginCustomerMode() {
        customerMode = true
        savedBrightness = UIScreen.main.brightness
        UIScreen.main.brightness = 1
        UIApplication.shared.isIdleTimerDisabled = true
    }
    /// Always puts brightness and sleep back, whichever way the QR screen is left.
    func endCustomerMode() {
        if let b = savedBrightness { UIScreen.main.brightness = b; savedBrightness = nil }
        UIApplication.shared.isIdleTimerDisabled = false
        customerMode = false
    }

    func start() {
        guard let amt = entry.money, let c = store.client else { return }
        phase = .creating
        Task {
            do {
                let intent = try await c.createIntent(amount: amt, note: note)
                let attempt = try await c.createUPIAttempt(intentId: intent.id)
                guard let uri = attempt.upiURI else { phase = .error("This provider did not issue a UPI QR. Is upi_intent enabled?"); return }
                phase = .waiting(intentId: intent.id, attemptId: attempt.id, uri: uri, amount: amt, started: Date())
            } catch { phase = .error(error.localizedDescription) }
        }
    }

    /// Ask the server for the truth, with backoff. A failed poll is "still checking", never a failure.
    func poll(_ attemptId: String, _ amt: Money) async {
        guard let c = store.client else { return }
        var delay: UInt64 = 1_500_000_000
        while !Task.isCancelled {
            if let a = try? await c.refreshAttempt(id: attemptId), a.isFinal {
                endCustomerMode()
                phase = a.isPaid ? .paid(amt) : .failed(a.failureReason ?? "Declined")
                a.isPaid ? Haptics.success() : Haptics.error()
                return
            }
            try? await Task.sleep(nanoseconds: delay)
            delay = min(delay + 500_000_000, 5_000_000_000)
        }
    }
}
