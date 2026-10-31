import LocalAuthentication
import SwiftUI
import VayoraKit
import VayoraUI

struct SettingsView: View {
    @EnvironmentObject var store: Store
    @AppStorage("faceIDLock") private var lock = false
    @State private var confirmReset = false

    var body: some View {
        NavigationStack {
            ZStack {
                AmbientBackground(brand: .pay, phase: 0.5)
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Text("Settings").vfont(28, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("Test server", systemImage: "server.rack").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                TextField("http://192.168.1.10:8080", text: $store.serverURL)
                                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL).foregroundStyle(VUI.textPrimary)
                                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                Text("Needs a Vayora API started with ZYVOR_SIM_PAYER=1.").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 12) {
                                Label("Security", systemImage: "lock.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Toggle("Lock with Face ID", isOn: $lock).tint(VUI.violet).foregroundStyle(VUI.textPrimary)
                                SecureField("Test PIN (4 digits)", text: $store.testPIN).keyboardType(.numberPad).foregroundStyle(VUI.textPrimary)
                                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                Text("A local test PIN, not a UPI PIN.").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 8) {
                                Label("About", systemImage: "info.circle.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Text("Vayora Pay is a test payer. It cannot link a bank or move real money. A real UPI app needs NPCI approval and a sponsor bank.")
                                    .vfont(14).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        Button(role: .destructive) { confirmReset = true } label: {
                            Text("Reset test wallet").vfont(17, .semibold).foregroundStyle(VUI.coral).frame(maxWidth: .infinity).padding(.vertical, 16)
                                .background(VUI.coral.opacity(0.14), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                                .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.coral.opacity(0.35)))
                        }.buttonStyle(PressScale())
                    }
                    .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
                }
            }
            .environment(\.colorScheme, .dark)
            .toolbar(.hidden, for: .navigationBar)
            .confirmationDialog("Reset balance to ₹10,000 and clear history?", isPresented: $confirmReset, titleVisibility: .visible) {
                Button("Reset", role: .destructive) { store.resetWallet() }
            }
        }
    }
}
