import LocalAuthentication
import SwiftUI
import VayoraKit

struct PaySheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    let payload: UPIPayload

    enum Step { case review, pin, working, done(Store.Outcome) }
    @State private var step: Step = .review
    @State private var typed = ""
    @State private var pin = ""
    @State private var pinError = false

    var amount: Money? { payload.amount ?? Money(decimal: typed) }

    var body: some View {
        NavigationStack {
            Group {
                switch step {
                case .review: review
                case .pin: pinEntry
                case .working: ProgressView("Paying…").controlSize(.large)
                case .done(let o): result(o)
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.canvas)
            .navigationTitle("Pay")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { if case .done = step {} else { Button("Cancel") { dismiss() } } } }
        }
        .interactiveDismissDisabled({ if case .working = step { return true } else { return false } }())
    }

    var review: some View {
        VStack(spacing: 20) {
            TestModeChip()
            Image(systemName: "storefront.fill").font(.system(size: 44)).foregroundStyle(Theme.accent)
            Text(payload.name.isEmpty ? payload.payee : payload.name).font(.title2.bold())
            Text(payload.payee).font(.footnote).foregroundStyle(.secondary)
            if let a = payload.amount {
                Text(a.display).font(.system(size: 44, weight: .bold, design: .rounded))
            } else {
                TextField("Amount", text: $typed).keyboardType(.decimalPad).font(.system(size: 44, weight: .bold, design: .rounded)).multilineTextAlignment(.center)
            }
            Text("Test balance \(store.balance.display)").font(.footnote).foregroundStyle(.secondary)
            Spacer()
            Button("Pay \(amount?.display ?? "")") { pin = ""; step = .pin }
                .buttonStyle(PrimaryButtonStyle()).disabled(amount == nil)
        }
    }

    var pinEntry: some View {
        VStack(spacing: 20) {
            Text("Enter your test PIN").font(.title3.bold())
            Text("This is a local test PIN, not a UPI PIN. Default 1234.").font(.footnote).foregroundStyle(.secondary)
            HStack(spacing: 14) {
                ForEach(0..<4, id: \.self) { i in
                    Circle().fill(i < pin.count ? Theme.accent : Color.secondary.opacity(0.3)).frame(width: 18, height: 18)
                }
            }
            .modifier(Shake(trigger: pinError))
            Spacer()
            keypad
        }
    }

    var keypad: some View {
        let keys = ["1","2","3","4","5","6","7","8","9","","0","⌫"]
        return LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 3), spacing: 12) {
            ForEach(keys, id: \.self) { k in
                if k.isEmpty { Color.clear.frame(height: 56) } else {
                    Button { tap(k) } label: {
                        Text(k).font(.title.weight(.medium)).frame(maxWidth: .infinity, minHeight: 56)
                            .background(Theme.card, in: RoundedRectangle(cornerRadius: 14))
                    }.buttonStyle(.plain).accessibilityLabel(k == "⌫" ? "Delete" : k)
                }
            }
        }
    }

    func tap(_ k: String) {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        if k == "⌫" { _ = pin.popLast(); return }
        guard pin.count < 4 else { return }
        pin += k
        if pin.count == 4 {
            if pin == store.testPIN { submit() } else {
                pinError.toggle(); UINotificationFeedbackGenerator().notificationOccurred(.error)
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { pin = "" }
            }
        }
    }

    func submit() {
        guard let amount else { return }
        step = .working
        Task {
            let outcome = await store.pay(payload, amount: amount)
            let status: Txn.Status = outcome == .paid ? .success : .failed
            if outcome != .unknown && outcome != .notAvailable {
                store.record(Txn(payee: payload.payee, name: payload.name, amountMinor: amount.minor, note: payload.note, status: status))
            }
            UINotificationFeedbackGenerator().notificationOccurred(outcome == .paid ? .success : .error)
            step = .done(outcome)
        }
    }

    @ViewBuilder func result(_ o: Store.Outcome) -> some View {
        VStack(spacing: 16) {
            switch o {
            case .paid:
                Image(systemName: "checkmark.circle.fill").font(.system(size: 88)).foregroundStyle(Theme.success).symbolEffect(.bounce, value: true)
                Text("Paid \(amount?.display ?? "")").font(.title.bold())
                Text("to \(payload.name.isEmpty ? payload.payee : payload.name)").foregroundStyle(.secondary)
            case .declined(let why):
                Image(systemName: "xmark.circle.fill").font(.system(size: 88)).foregroundStyle(Theme.danger)
                Text("Payment failed").font(.title.bold())
                Text(why).foregroundStyle(.secondary).multilineTextAlignment(.center)
            case .unknown:
                Image(systemName: "questionmark.circle.fill").font(.system(size: 88)).foregroundStyle(Theme.warn)
                Text("Checking…").font(.title.bold())
                Text("We couldn't confirm this payment. Don't pay again: ask the merchant if it arrived.")
                    .foregroundStyle(.secondary).multilineTextAlignment(.center)
                if !store.lastError.isEmpty {
                    Text("Details: \(store.lastError)").font(.caption2).foregroundStyle(.secondary)
                        .multilineTextAlignment(.center).textSelection(.enabled)
                }
            case .notAvailable:
                Image(systemName: "wifi.exclamationmark").font(.system(size: 72)).foregroundStyle(Theme.warn)
                Text("Test server not available").font(.title2.bold())
                Text("Check the server address in Settings, and that the server was started with ZYVOR_SIM_PAYER=1.")
                    .foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
            Spacer()
            Button("Done") { dismiss() }.buttonStyle(PrimaryButtonStyle())
        }
    }
}

struct Shake: ViewModifier {
    var trigger: Bool
    func body(content: Content) -> some View {
        content.offset(x: trigger ? 0 : 0).animation(nil, value: trigger)
            .keyframeAnimator(initialValue: 0.0, trigger: trigger) { v, x in v.offset(x: x) } keyframes: { _ in
                KeyframeTrack { LinearKeyframe(10, duration: 0.07); LinearKeyframe(-10, duration: 0.07); LinearKeyframe(6, duration: 0.07); LinearKeyframe(0, duration: 0.07) }
            }
    }
}
