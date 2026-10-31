import SwiftUI
import VayoraKit
import VayoraUI

/// Split a bill: a total, how many people, then one request QR that each friend scans in their own Vayora Pay.
struct SplitView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var entry = AmountEntry()
    @State private var people = 2
    @State private var step = 0   // 0 total, 1 people, 2 QR

    /// The share rounds up to the next paisa, so the shares never add up to less than the bill.
    func share(_ total: Money) -> Money { Money(minor: (total.minor + Int64(people) - 1) / Int64(people)) }

    var body: some View {
        Group {
            switch step {
            case 0:
                CollectKeypadScreen(text: entry.text, note: "", chips: [], canContinue: entry.money != nil, title: "Split a bill", actionLabel: "Next", actionIcon: "arrow.right", brand: .pay,
                                    onDigit: { entry.digit($0) }, onPoint: { entry.point() }, onDelete: { entry.delete() }, onShowQR: { withAnimation(VUI.spring) { step = 1 } })
            case 1:
                if let total = entry.money { peopleStep(total) }
            default:
                if let total = entry.money {
                    let s = share(total)
                    SplitShareScreen(total: total, people: people, share: s, qr: QR.image(RequestPayload.make(payee: store.myVPA, name: store.myName, amount: s, note: "Split")),
                                     myName: store.myName, onDone: { dismiss() })
                }
            }
        }
        .background(VUI.ink).preferredColorScheme(.dark)
    }

    func peopleStep(_ total: Money) -> some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.66)
            VStack(spacing: 22) {
                HStack { Text("How many people?").vfont(24, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
                Spacer()
                AmountText(total, size: 48)
                HStack(spacing: 26) {
                    stepper("minus", "Fewer people") { if people > 2 { people -= 1; Haptics.tick() } }
                    Text("\(people)").vfont(64, .bold).foregroundStyle(VUI.textPrimary).contentTransition(.numericText()).animation(VUI.snap, value: people).frame(minWidth: 90)
                    stepper("plus", "More people") { if people < 20 { people += 1; Haptics.tick() } }
                }
                Text("Each pays \(share(total).display)").vfont(18, .semibold).foregroundStyle(VUI.mint)
                if share(total).minor * Int64(people) != total.minor { Text("Rounded up to the next paisa, so you collect \(Money(minor: share(total).minor * Int64(people) - total.minor).display) extra.").vfont(12).foregroundStyle(VUI.textTertiary).multilineTextAlignment(.center) }
                Spacer()
                Button { withAnimation(VUI.spring) { step = 2 } } label: {
                    Text("Show request QR").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                        .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                }.buttonStyle(PressScale())
            }
            .padding(24)
        }
    }

    func stepper(_ icon: String, _ label: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            Image(systemName: icon).font(.system(size: 24, weight: .bold)).foregroundStyle(.white).frame(width: 64, height: 64)
                .background(.white.opacity(0.12), in: Circle()).overlay(Circle().strokeBorder(VUI.hairline))
        }.buttonStyle(PressScale()).accessibilityLabel(label)
    }
}
