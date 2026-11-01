import SwiftUI
import VayoraKit
import VayoraUI

/// Monthly limits per category. They warn before a payment; they never stop one.
struct BudgetsView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var text: [SpendCategory: String] = [:]

    static let editable: [SpendCategory] = [.food, .shopping, .bills, .travel, .other]

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.7)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    HStack {
                        Text("Budgets").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    Text("A monthly limit per category. Leave it empty to not watch one. You get a warning on the review screen near the limit; nothing is ever blocked.").vfont(14).foregroundStyle(VUI.textSecondary)
                    ForEach(Self.editable, id: \.self) { c in
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                HStack {
                                    Text(c.title).vfont(17, .semibold).foregroundStyle(VUI.textPrimary); Spacer()
                                    if let s = store.budgetStatuses.first(where: { $0.category == c }) {
                                        Text("\(Money(minor: s.spentMinor).display) spent").vfont(13).foregroundStyle(colour(s.level))
                                    }
                                }
                                if let s = store.budgetStatuses.first(where: { $0.category == c }) {
                                    ProgressView(value: min(1, s.fraction)).tint(colour(s.level))
                                }
                                TextField("Limit in ₹ a month", text: binding(c)).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary)
                                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            }
                        }
                    }
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .onAppear { for c in Self.editable { if let l = store.budgets.limit(c) { text[c] = Money(minor: l).decimalString } } }
    }

    func binding(_ c: SpendCategory) -> Binding<String> {
        Binding(get: { text[c] ?? "" }, set: { v in
            text[c] = v
            store.budgets.set(c, minor: Money(decimal: v)?.minor)
        })
    }
    func colour(_ l: BudgetStatus.Level) -> Color { switch l { case .ok: VUI.mint; case .near: VUI.amber; case .over: VUI.coral } }
}

/// "Pay me": a QR with your test UPI ID and no amount. Another Vayora Pay scans it, types the amount, and pays.
struct ReceiveView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    var link: String { RequestPayload.makeOpen(payee: store.myVPA, name: store.myName) }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.2)
            VStack(spacing: 18) {
                HStack {
                    Text("Receive money").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                    Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                }
                Spacer()
                QR.image(link)?.interpolation(.none).resizable().scaledToFit().frame(maxWidth: 280).padding(16).background(.white, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                    .accessibilityLabel("Your payment QR")
                Text(store.myName.isEmpty ? "Your test UPI ID" : store.myName).vfont(20, .semibold).foregroundStyle(VUI.textPrimary)
                Text(store.myVPA).vfont(14).foregroundStyle(VUI.textSecondary).textSelection(.enabled)
                ShareLink(item: link) { Label("Share link", systemImage: "square.and.arrow.up").vfont(16, .semibold).foregroundStyle(VUI.violet) }
                Spacer()
                Text("Test mode: another Vayora Pay can pay this. The money moves between test wallets on the phones; Vayora's ledger is not involved.").vfont(12).foregroundStyle(VUI.textTertiary).multilineTextAlignment(.center)
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}
