import SwiftUI
import VayoraKit
import VayoraUI

/// Amount (when the QR has none), review and slide, PIN, then the result. All the looks live in VayoraUI.
struct PaySheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    let payload: UPIPayload

    enum Step: Equatable { case amount, review, pin, working, done(Store.Outcome) }
    @State private var step: Step
    @State private var entry = AmountEntry()
    @State private var pin = PinEntry()
    @State private var pinError = false
    @State private var shake: CGFloat = 0
    @State private var progress = 0.0

    init(payload: UPIPayload) {
        self.payload = payload
        _step = State(initialValue: payload.amount == nil ? .amount : .review)
    }

    var amount: Money? { payload.amount ?? entry.money }
    var payeeName: String { payload.name.isEmpty ? payload.payee : payload.name }

    var body: some View {
        ZStack(alignment: .topLeading) {
            switch step {
            case .amount:
                CollectKeypadScreen(text: entry.text, note: "", chips: [], canContinue: amount != nil, title: "Pay \(payeeName)", actionLabel: "Continue", actionIcon: "arrow.right", brand: .pay,
                                    onDigit: { entry.digit($0) }, onPoint: { entry.point() }, onDelete: { entry.delete() }, onShowQR: { withAnimation(VUI.spring) { step = .review } })
            case .review:
                if let amount {
                    PayReviewScreen(name: payload.name, payee: payload.payee, amount: amount, balance: store.balance) {
                        pin.clear(); pinError = false
                        withAnimation(VUI.spring) { step = .pin }
                    }
                }
            case .pin:
                PinScreen(filled: pin.digits.count, error: pinError, shake: shake, onDigit: { digit($0) }, onDelete: { pin.delete(); pinError = false })
            case .working:
                ZStack {
                    AmbientBackground(brand: .pay)
                    VStack(spacing: 18) { ProgressView().controlSize(.large).tint(.white); Text("Paying…").font(VUI.display(22)).foregroundStyle(VUI.textPrimary) }
                }
            case .done(let o):
                ResultScreen(kind: kind(o), amount: amount ?? Money(minor: 0), to: payeeName, progress: progress, brand: .pay, footnote: footnote(o), onDone: { dismiss() })
            }
            if closable {
                Button { dismiss() } label: {
                    Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle())
                }
                .padding(.leading, 18).padding(.top, 14).accessibilityLabel("Close")
            }
        }
        .background(VUI.ink)
        .preferredColorScheme(.dark)
        .interactiveDismissDisabled(step == .working)
    }

    var closable: Bool { switch step { case .working, .done: false; default: true } }

    func digit(_ d: Int) {
        pin.press(d)
        guard pin.isFull else { return }
        if pin.code == store.testPIN { submit() } else {
            pinError = true; Haptics.error()
            Task {
                for x: CGFloat in [12, -12, 8, -8, 0] { withAnimation(.linear(duration: 0.06)) { shake = x }; try? await Task.sleep(nanoseconds: 60_000_000) }
                try? await Task.sleep(nanoseconds: 250_000_000)
                pin.clear()
            }
        }
    }

    func submit() {
        guard let amount else { return }
        withAnimation(VUI.spring) { step = .working }
        Task {
            let outcome = await store.pay(payload, amount: amount)
            if outcome != .unknown && outcome != .notAvailable {
                store.record(Txn(payee: payload.payee, name: payload.name, amountMinor: amount.minor, note: payload.note, status: outcome == .paid ? .success : .failed))
            }
            progress = 0
            withAnimation(.easeInOut(duration: 0.3)) { step = .done(outcome) }
            outcome == .paid ? Haptics.success() : Haptics.error()
            withAnimation(.easeOut(duration: 1.3)) { progress = 1 }
        }
    }

    func kind(_ o: Store.Outcome) -> ResultKind {
        switch o {
        case .paid: .paid
        case .declined(let why): .failed(why)
        case .unknown: .checking
        case .notAvailable: .failed("Test server not available. Check the address in Settings, and that the server runs with ZYVOR_SIM_PAYER=1.")
        }
    }
    func footnote(_ o: Store.Outcome) -> String? { o == .unknown && !store.lastError.isEmpty ? "Details: \(store.lastError)" : nil }
}
