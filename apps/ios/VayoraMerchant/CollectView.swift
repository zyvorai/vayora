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
    @State private var tipPercent = 0
    @State private var unlocking = false
    @AppStorage("lastAmountMinor") private var lastMinor = 0

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
                    staffRow
                    helpers
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
                .overlay(alignment: .bottomTrailing) {
                    // send the QR to a customer who is not at the counter; the amount is in the image's caption
                    if let img = QR.image(uri) {
                        ShareLink(item: img, preview: SharePreview("Pay \(amt.display)", image: img)) {
                            Label("Share QR", systemImage: "square.and.arrow.up").vfont(14, .semibold).foregroundStyle(VUI.ink).padding(.horizontal, 14).padding(.vertical, 10).background(VUI.mint, in: Capsule())
                        }.padding(20).accessibilityHint("Shares the payment QR as an image")
                    }
                }
            case .paid(let m):
                ResultScreen(kind: .paid, amount: m, to: note.isEmpty ? "this payment" : note, progress: resultProgress, brand: .merchant, paidTitle: "Received", doneLabel: "New payment", onDone: reset)
            case .failed(let why):
                ResultScreen(kind: .failed(why), amount: Money(minor: 0), to: "", progress: 1, brand: .merchant, doneLabel: "New payment", onDone: reset)
            case .error(let why):
                ResultScreen(kind: .failed(why), amount: Money(minor: 0), to: "", progress: 1, brand: .merchant, doneLabel: "Back", onDone: reset)
            }
        }
        .background(VUI.ink)
        .overlay(alignment: .topTrailing) {
            if store.cashierLocked, phase == .entering {
                Button { unlocking = true } label: { Label("Owner", systemImage: "lock.fill").vfont(13, .semibold).foregroundStyle(VUI.textSecondary).padding(.horizontal, 12).padding(.vertical, 7).background(.white.opacity(0.1), in: Capsule()) }
                    .padding(.trailing, 16).padding(.top, 6)
            }
        }
        .sheet(isPresented: $unlocking) { OwnerUnlock().environmentObject(store) }
        .animation(VUI.spring, value: phase)
        .onChange(of: phase) { _, new in
            if case .paid = new { resultProgress = 0; withAnimation(.easeOut(duration: 1.3)) { resultProgress = 1 } }
        }
        .onDisappear { endCustomerMode() }
    }

    /// Repeat the last amount, and an optional tip on top. The tip is part of the payment, so the QR asks for the total.
    @ViewBuilder var staffRow: some View {
        if !store.staff.isEmpty, phase == .entering {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    Text("On the till").vfont(12, .semibold).foregroundStyle(VUI.textTertiary)
                    ForEach(store.staff, id: \.self) { n in
                        Button { store.activeStaff = (store.activeStaff == n ? nil : n); Haptics.tick() } label: { chip(n, on: store.activeStaff == n) }.buttonStyle(PressScale())
                            .accessibilityAddTraits(store.activeStaff == n ? .isSelected : [])
                    }
                }.padding(.horizontal, 20)
            }
            .padding(.vertical, 6).background(VUI.ink)
        }
    }
    @ViewBuilder var helpers: some View {
        let base = entry.money?.minor ?? 0
        if base > 0 || lastMinor > 0 {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    if base == 0, lastMinor > 0 {
                        Button { repeatLast() } label: { chip("Repeat \(Money(minor: Int64(lastMinor)).display)", on: false) }.buttonStyle(PressScale())
                    }
                    if base > 0 {
                        ForEach([0] + Tip.percents, id: \.self) { p in
                            Button { tipPercent = p; Haptics.tick() } label: {
                                chip(p == 0 ? "No tip" : "Tip \(p)% · \(Money(minor: Tip.amount(base: base, percent: p)).display)", on: tipPercent == p)
                            }.buttonStyle(PressScale()).accessibilityAddTraits(tipPercent == p ? .isSelected : [])
                        }
                    }
                }.padding(.horizontal, 20)
            }
            .padding(.vertical, 8).background(VUI.ink)
        }
    }
    func chip(_ t: String, on: Bool) -> some View {
        Text(t).vfont(13, .semibold).foregroundStyle(on ? VUI.ink : VUI.textSecondary).padding(.horizontal, 14).padding(.vertical, 8)
            .background(on ? AnyShapeStyle(VUI.emerald) : AnyShapeStyle(.white.opacity(0.08)), in: Capsule())
    }
    func repeatLast() {
        entry.clear()
        for ch in Money(minor: Int64(lastMinor)).decimalString { if let d = ch.wholeNumberValue { entry.digit(d) } else if ch == "." { entry.point() } }
        Haptics.tick()
    }

    func reset() {
        endCustomerMode(); entry.clear(); note = ""; tipPercent = 0; phase = .entering
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
        guard let base = entry.money, let c = store.client else { return }
        let tip = Tip.amount(base: base.minor, percent: tipPercent)
        let amt = Money(minor: base.minor + tip)
        if tip > 0 { note = (note.isEmpty ? "" : note + " · ") + "incl. tip \(Money(minor: tip).display)" }
        lastMinor = Int(base.minor)
        phase = .creating
        Task {
            do {
                let intent = try await c.createIntent(amount: amt, note: note)
                if let who = store.activeStaff { store.staffTags.tag(intent.id, as: who) }
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
                if a.isPaid { Soundbox.shared.announce(amt) }
                phase = a.isPaid ? .paid(amt) : .failed(a.failureReason ?? "Declined")
                a.isPaid ? Haptics.success() : Haptics.error()
                return
            }
            try? await Task.sleep(nanoseconds: delay)
            delay = min(delay + 500_000_000, 5_000_000_000)
        }
    }
}
