import LocalAuthentication
import SwiftUI
import VayoraKit

struct SettingsView: View {
    @EnvironmentObject var store: Store
    @AppStorage("faceIDLock") private var lock = false
    @State private var confirmReset = false
    var body: some View {
        NavigationStack {
            Form {
                Section("Test server") {
                    TextField("http://192.168.1.10:8080", text: $store.serverURL)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                    Text("Needs a Vayora API started with ZYVOR_SIM_PAYER=1.").font(.footnote).foregroundStyle(.secondary)
                }
                Section("Security") {
                    Toggle("Lock with Face ID", isOn: $lock)
                    SecureField("Test PIN (4 digits)", text: $store.testPIN).keyboardType(.numberPad)
                }
                Section("About") {
                    LabeledContent("Mode", value: "Simulator, test money")
                    Text("Vayora Pay is a test payer. It cannot link a bank or move real money. A real UPI app needs NPCI approval and a sponsor bank.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                Section { Button("Reset test wallet", role: .destructive) { confirmReset = true } }
            }
            .navigationTitle("Settings")
            .confirmationDialog("Reset balance to ₹10,000 and clear history?", isPresented: $confirmReset) {
                Button("Reset", role: .destructive) { store.resetWallet() }
            }
        }
    }
}
