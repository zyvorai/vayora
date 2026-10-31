import SwiftUI
import VayoraKit
import VayoraUI

/// Review "up to ₹X, how often, until when", slide, PIN, then the simulated bank turns the AutoPay on.
struct MandateApproveSheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    let mandate: MandatePayload
    let uri: String

    enum Step: Equatable { case review, pin, working, done, failed(String) }
    @State private var step = Step.review
    @State private var pin = PinEntry()
    @State private var pinError = false
    @State private var shake: CGFloat = 0
    @State private var progress = 0.0

    var until: String? { mandate.end?.formatted(.dateTime.day().month(.abbreviated).year()) }

    var body: some View {
        ZStack(alignment: .topLeading) {
            switch step {
            case .review:
                MandateReviewScreen(merchant: mandate.name, payee: mandate.payee, cap: mandate.cap, frequency: mandate.frequency.title, until: until) {
                    pin.clear(); pinError = false
                    withAnimation(VUI.spring) { step = .pin }
                }
            case .pin:
                PinScreen(filled: pin.digits.count, error: pinError, shake: shake, onDigit: { digit($0) }, onDelete: { pin.delete(); pinError = false })
            case .working:
                ZStack {
                    AmbientBackground(brand: .pay)
                    VStack(spacing: 18) { ProgressView().controlSize(.large).tint(.white); Text("Turning on AutoPay…").vfont(22, .bold).foregroundStyle(VUI.textPrimary) }
                }
            case .done:
                ResultScreen(kind: .paid, amount: mandate.cap, to: mandate.name.isEmpty ? mandate.payee : mandate.name, progress: progress, brand: .pay, paidTitle: "AutoPay on",
                             footnote: "AutoPay is on. Up to \(mandate.cap.display), \(mandate.frequency.title.lowercased()). Nothing is taken now. Stop it any time under Bills.", onDone: { dismiss() })
            case .failed(let why):
                ResultScreen(kind: .failed(why), amount: mandate.cap, to: mandate.name.isEmpty ? mandate.payee : mandate.name, progress: 1, brand: .pay, footnote: nil, onDone: { dismiss() })
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

    var closable: Bool { switch step { case .working, .done, .failed: false; default: true } }

    func digit(_ d: Int) {
        pin.press(d)
        guard pin.isFull else { return }
        if pin.code == store.testPIN { approve() } else {
            pinError = true; Haptics.error()
            Task {
                for x: CGFloat in [12, -12, 8, -8, 0] { withAnimation(.linear(duration: 0.06)) { shake = x }; try? await Task.sleep(nanoseconds: 60_000_000) }
                try? await Task.sleep(nanoseconds: 250_000_000)
                pin.clear()
            }
        }
    }

    func approve() {
        withAnimation(VUI.spring) { step = .working }
        Task {
            switch await store.approveMandate(mandate, uri: uri) {
            case .success:
                progress = 0
                withAnimation(.easeInOut(duration: 0.3)) { step = .done }
                Haptics.success()
                withAnimation(.easeOut(duration: 1.3)) { progress = 1 }
            case .failure(let e):
                withAnimation(.easeInOut(duration: 0.3)) { step = .failed(e.message) }
                Haptics.error()
            }
        }
    }
}
