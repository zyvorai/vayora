import SwiftUI
import VayoraKit

/// Create a payment, show its QR, and watch it complete.
struct CollectView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var amountText = ""
    @State private var note = ""
    @State private var phase: Phase = .entering

    enum Phase: Equatable {
        case entering, creating
        case waiting(intentId: String, attemptId: String, uri: String, amount: Money)
        case paid(Money), failed(String), error(String)
    }

    var amount: Money? { Money(decimal: amountText) }

    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                TestModeChip()
                switch phase {
                case .entering, .creating: entry
                case .waiting(let intent, let attempt, let uri, let amt): waiting(intent, attempt, uri, amt)
                case .paid(let m): outcome("checkmark.circle.fill", Theme.success, "Received \(m.display)")
                case .failed(let why): outcome("xmark.circle.fill", Theme.danger, "Payment failed", why)
                case .error(let why): outcome("exclamationmark.triangle.fill", Theme.warn, "Couldn't start", why)
                }
                Spacer()
            }
            .padding(20).background(Theme.canvas).navigationTitle("Collect")
        }
    }

    var entry: some View {
        VStack(spacing: 16) {
            TextField("₹0", text: $amountText).keyboardType(.decimalPad)
                .font(.system(size: 56, weight: .bold, design: .rounded)).multilineTextAlignment(.center)
            if !amountText.isEmpty && amount == nil { Text("Enter an amount like 499 or 499.50").font(.footnote).foregroundStyle(Theme.danger) }
            TextField("Note (optional)", text: $note).textFieldStyle(.roundedBorder)
            Button(phase == .creating ? "Creating…" : "Show QR") { start() }
                .buttonStyle(PrimaryButtonStyle()).disabled(amount == nil || phase == .creating)
            if store.client == nil { Text("Add your API key in Settings first.").font(.footnote).foregroundStyle(.secondary) }
        }
    }

    func waiting(_ intent: String, _ attempt: String, _ uri: String, _ amt: Money) -> some View {
        VStack(spacing: 14) {
            Text(amt.display).font(.system(size: 40, weight: .bold, design: .rounded))
            if let qr = QR.image(uri) {
                qr.resizable().frame(width: 260, height: 260).padding(14)
                    .background(.white, in: RoundedRectangle(cornerRadius: 20)).accessibilityLabel("Payment QR code")
            }
            HStack { ProgressView(); Text("Waiting for payment…").foregroundStyle(.secondary) }
            Button("Cancel", role: .cancel) { phase = .entering }
        }
        .task(id: attempt) { await poll(attempt, amt) }
    }

    func outcome(_ icon: String, _ color: Color, _ title: String, _ detail: String? = nil) -> some View {
        VStack(spacing: 14) {
            Image(systemName: icon).font(.system(size: 88)).foregroundStyle(color).symbolEffect(.bounce, value: true)
            Text(title).font(.title.bold())
            if let detail { Text(detail).foregroundStyle(.secondary).multilineTextAlignment(.center) }
            Button("New payment") { amountText = ""; note = ""; phase = .entering }.buttonStyle(PrimaryButtonStyle())
        }
    }

    func start() {
        guard let amt = amount, let c = store.client else { return }
        phase = .creating
        Task {
            do {
                let intent = try await c.createIntent(amount: amt, note: note)
                let attempt = try await c.createUPIAttempt(intentId: intent.id)
                guard let uri = attempt.upiURI else { phase = .error("This provider did not issue a UPI QR. Is upi_intent enabled?"); return }
                phase = .waiting(intentId: intent.id, attemptId: attempt.id, uri: uri, amount: amt)
            } catch { phase = .error(error.localizedDescription) }
        }
    }

    /// Ask the server for the truth, with backoff. A failed poll is "still checking", never a failure.
    func poll(_ attemptId: String, _ amt: Money) async {
        guard let c = store.client else { return }
        var delay: UInt64 = 1_500_000_000
        while !Task.isCancelled {
            if let a = try? await c.refreshAttempt(id: attemptId), a.isFinal {
                phase = a.isPaid ? .paid(amt) : .failed(a.failureReason ?? "Declined")
                UINotificationFeedbackGenerator().notificationOccurred(a.isPaid ? .success : .error)
                return
            }
            try? await Task.sleep(nanoseconds: delay)
            delay = min(delay + 500_000_000, 5_000_000_000)
        }
    }
}
