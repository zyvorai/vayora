import SwiftUI
import VayoraKit

struct PaymentsView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var items: [PaymentIntent] = []
    @State private var error: String?
    @State private var loading = false

    var todayTotal: Money {
        Money(minor: items.filter { $0.status == "succeeded" }.reduce(0) { $0 + $1.amountMinor })
    }

    var body: some View {
        NavigationStack {
            List {
                Section { VStack(alignment: .leading) { Text("Collected").font(.subheadline).foregroundStyle(.secondary); Text(todayTotal.display).font(.title.bold()) } }
                if let error { Text(error).foregroundStyle(Theme.danger) }
                ForEach(items) { i in
                    NavigationLink { PaymentDetail(intent: i) } label: {
                        HStack { VStack(alignment: .leading) { Text(i.money.display).font(.body.weight(.semibold)); Text(i.id).font(.caption2).foregroundStyle(.secondary) }
                            Spacer(); Text(i.status).font(.caption.weight(.semibold)).foregroundStyle(i.status == "succeeded" ? Theme.success : .secondary) }
                    }
                }
            }
            .overlay { if items.isEmpty && !loading && error == nil { ContentUnavailableView("No payments yet", systemImage: "tray") } }
            .refreshable { await load() }
            .task { await load() }
            .navigationTitle("Payments")
        }
    }

    func load() async {
        guard let c = store.client else { error = "Add your API key in Settings."; return }
        loading = true; defer { loading = false }
        do { items = try await c.listIntents(); error = nil } catch { self.error = error.localizedDescription }
    }
}

struct PaymentDetail: View {
    @EnvironmentObject var store: MerchantStore
    let intent: PaymentIntent
    @State private var message: String?
    @State private var refunding = false

    var body: some View {
        Form {
            LabeledContent("Amount", value: intent.money.display)
            LabeledContent("Status", value: intent.status)
            LabeledContent("ID", value: intent.id)
            if intent.status == "succeeded" {
                Section {
                    Button("Refund in full", role: .destructive) { refund() }.disabled(refunding)
                    if let message { Text(message).font(.footnote) }
                } footer: { Text("Test mode: the refund is recorded against the simulator.") }
            }
        }
        .navigationTitle("Payment")
    }

    func refund() {
        guard let c = store.client else { return }
        refunding = true
        Task {
            defer { refunding = false }
            do { let r = try await c.refund(intentId: intent.id, amount: nil, reason: "requested_by_merchant"); message = "Refund \(r.status)" }
            catch { message = error.localizedDescription }
        }
    }
}

struct MerchantSettings: View {
    @EnvironmentObject var store: MerchantStore
    var body: some View {
        NavigationStack {
            Form {
                Section("Server") {
                    TextField("https://api.example.com", text: $store.serverURL).textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                }
                Section("API key") {
                    SecureField("zy_test_…", text: $store.apiKey).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Text("Stored in the iPhone Keychain, never in app settings.").font(.footnote).foregroundStyle(.secondary)
                }
                Section("About") {
                    Text("Vayora Merchant creates test payments and shows their QR. With the simulator or Cashfree sandbox no real money moves.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }.navigationTitle("Settings")
        }
    }
}
