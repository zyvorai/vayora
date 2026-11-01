import LocalAuthentication
import UniformTypeIdentifiers
import SwiftUI
import VayoraKit
import VayoraUI

struct SettingsView: View {
    @EnvironmentObject var store: Store
    @AppStorage("faceIDLock") private var lock = false
    @AppStorage("lockTimeout") private var lockTimeout = 0
    @AppStorage("hideBalance") private var hideBalance = false
    @State private var backupURL: URL?
    @State private var importing = false
    @State private var backupMessage: String?
    @State private var confirmReset = false
    @State private var scanning = false
    @State private var budgeting = false
    @State private var receiving = false
    @State private var jarring = false
    @State private var setupMessage: String?

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
                                Button { scanning = true } label: { Label("Scan setup QR", systemImage: "qrcode.viewfinder") }
                                    .vfont(15, .semibold).foregroundStyle(VUI.violet)
                                if let setupMessage { Text(setupMessage).vfont(13).foregroundStyle(VUI.textSecondary) }
                                Text("Needs a Vayora API started with ZYVOR_SIM_PAYER=1.").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 12) {
                                Label("Security", systemImage: "lock.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Toggle("Lock with Face ID", isOn: $lock).tint(VUI.violet).foregroundStyle(VUI.textPrimary)
                                if lock {
                                    Picker("Ask again", selection: $lockTimeout) { Text("Every time").tag(0); Text("After 1 min").tag(1); Text("After 5 min").tag(5) }.pickerStyle(.segmented)
                                }
                                Toggle("Hide the balance", isOn: $hideBalance).tint(VUI.violet).foregroundStyle(VUI.textPrimary)
                                Text("The app is also blurred in the app switcher.").vfont(12).foregroundStyle(VUI.textTertiary)
                                SecureField("Test PIN (4 digits)", text: $store.testPIN).keyboardType(.numberPad).foregroundStyle(VUI.textPrimary)
                                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                Text("A local test PIN, not a UPI PIN.").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("Budgets and receiving", systemImage: "chart.pie.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                let s = store.budgetStatuses
                                Text(s.isEmpty ? "No limits set." : s.map { "\($0.category.title) \(Int($0.fraction * 100))%" }.joined(separator: " · ")).vfont(13).foregroundStyle(VUI.textSecondary)
                                HStack(spacing: 18) {
                                    Button { budgeting = true } label: { Label("Budgets", systemImage: "slider.horizontal.3") }
                                    Button { receiving = true } label: { Label("Receive", systemImage: "qrcode") }
                                    Button { jarring = true } label: { Label("Jars", systemImage: "banknote") }
                                }.vfont(15, .semibold).foregroundStyle(VUI.violet)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 12) {
                                Label("Small payments", systemImage: "bolt.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Toggle("No PIN up to ₹200", isOn: $store.smallPayments).tint(VUI.violet).foregroundStyle(VUI.textPrimary)
                                Text("Test limit: ₹200 a payment and ₹1,000 a day without a PIN. Above that, the PIN is still asked.").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("You", systemImage: "person.crop.circle.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                TextField("Your name", text: $store.myName).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                Text("Your test UPI ID: \(store.myVPA)").vfont(13).foregroundStyle(VUI.textSecondary).textSelection(.enabled)
                                Text("Split requests ask to be paid here.").vfont(12).foregroundStyle(VUI.textTertiary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("Backup", systemImage: "externaldrive.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                HStack(spacing: 18) {
                                    if let backupURL { ShareLink(item: backupURL) { Label("Share backup", systemImage: "square.and.arrow.up") } }
                                    else { Button { if let d = store.exportBackup() { let u = FileManager.default.temporaryDirectory.appendingPathComponent("vayora-pay-backup.json"); try? d.write(to: u, options: .atomic); backupURL = u } } label: { Label("Make a backup", systemImage: "square.and.arrow.down") } }
                                    Button { importing = true } label: { Label("Restore", systemImage: "arrow.counterclockwise") }
                                }.vfont(15, .semibold).foregroundStyle(VUI.violet)
                                if let backupMessage { Text(backupMessage).vfont(13).foregroundStyle(VUI.textSecondary) }
                                Text("Payees, history, jars, budgets, notes and the wallet. Not the PIN, the server address or receipt photos. Restoring replaces what is on this phone.").vfont(12).foregroundStyle(VUI.textTertiary)
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
            .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
                guard case .success(let url) = result else { return }
                let ok = url.startAccessingSecurityScopedResource(); defer { if ok { url.stopAccessingSecurityScopedResource() } }
                guard let data = try? Data(contentsOf: url) else { backupMessage = "Couldn't read that file."; return }
                switch store.restoreBackup(data) {
                case .success(let n): backupMessage = "Restored \(n) items."; backupURL = nil; Haptics.success()
                case .failure(.tooBig): backupMessage = "That file is too big to be a backup."; Haptics.error()
                case .failure(.wrongVersion): backupMessage = "That backup is from a different version."; Haptics.error()
                case .failure: backupMessage = "That is not a Vayora Pay backup."; Haptics.error()
                }
            }
            .sheet(isPresented: $budgeting) { BudgetsView().environmentObject(store) }
            .sheet(isPresented: $receiving) { ReceiveView().environmentObject(store) }
            .sheet(isPresented: $jarring) { JarsView().environmentObject(store) }
            .fullScreenCover(isPresented: $scanning) {
                ScannerView(brand: .pay, hint: "Scan the setup QR") { text in
                    scanning = false
                    if let p = try? SetupPayload.parse(text) {
                        store.serverURL = p.server.absoluteString
                        setupMessage = "Set up for \(p.server.host ?? "server")."
                        Haptics.success()
                    } else { setupMessage = "That is not a Vayora setup QR." }
                } onCancel: { scanning = false }
            }
            .confirmationDialog("Reset balance to ₹10,000 and clear history?", isPresented: $confirmReset, titleVisibility: .visible) {
                Button("Reset", role: .destructive) { store.resetWallet() }
            }
        }
    }
}
