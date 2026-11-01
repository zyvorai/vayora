import SwiftUI
import VayoraKit
import VayoraUI

/// Running totals per name you tagged on payments. Records only: nothing is lent, charged or sent.
struct RegularsView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var totals: [RegularsBook.Total] = []
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.3)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Regulars").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    if totals.isEmpty { GlassCard { Text("Open a paid payment and give it a name. Totals per name show here.").vfont(14).foregroundStyle(VUI.textSecondary) } }
                    ForEach(totals, id: \.name) { t in
                        GlassCard {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) { Text(t.name).vfont(17, .semibold).foregroundStyle(VUI.textPrimary); Text("\(t.count) payment\(t.count == 1 ? "" : "s")").vfont(13).foregroundStyle(VUI.textSecondary) }
                                Spacer()
                                Text(Money(minor: t.minor).display).vfont(17, .semibold).foregroundStyle(VUI.mint)
                            }
                        }
                    }
                    if let message { Text(message).vfont(13).foregroundStyle(VUI.textSecondary) }
                    Text("Names live on this phone only. Refunds are not subtracted here.").vfont(12).foregroundStyle(VUI.textTertiary)
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .task { await load() }
    }

    func load() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; return }
        do {
            let items = try await c.listIntents()
            totals = store.regulars.totals(items.map { ($0.id, $0.amountMinor, $0.status == "succeeded") })
        } catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
    }
}

/// The owner's PIN, to leave cashier mode.
struct OwnerUnlock: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var pin = ""
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.7)
            VStack(alignment: .leading, spacing: 16) {
                Text("Owner PIN").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                SecureField("PIN", text: $pin).keyboardType(.numberPad).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                if let message { Text(message).vfont(14).foregroundStyle(VUI.coral) }
                Button(action: unlock) { Text("Unlock").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 16).background(Brand.merchant.gradient, in: RoundedRectangle(cornerRadius: 22, style: .continuous)) }
                    .buttonStyle(PressScale()).opacity(pin.count >= 4 ? 1 : 0.4).disabled(pin.count < 4)
                Button("Cancel") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity)
                Spacer()
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }

    func unlock() {
        let now = Date()
        if store.ownerPIN.isEmpty { store.cashierLocked = false; dismiss(); return }   // no PIN was ever set: never trap the owner out
        if store.gate.attempt(pin, against: store.ownerPIN, now: now) { store.cashierLocked = false; Haptics.success(); dismiss(); return }
        pin = ""; Haptics.error()
        message = store.gate.isLocked(now: now) ? "Too many tries. Wait \(store.gate.secondsLeft(now: now)) seconds." : "That is not the PIN."
    }
}
