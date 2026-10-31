import SwiftUI
import VayoraKit

struct PaymentsView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var items: [PaymentIntent] = []
    @State private var refunds: [String: RefundState] = [:]
    @State private var error: String?
    @State private var loading = false

    /// Money kept after refunds: paid payments minus what has been given back.
    var collected: Money {
        let paid = items.filter { $0.status == "succeeded" }
        let back = paid.reduce(Int64(0)) { $0 + (refunds[$1.id]?.refundedMinor ?? 0) }
        return Money(minor: paid.reduce(Int64(0)) { $0 + $1.amountMinor } - back)
    }

    func badge(_ i: PaymentIntent) -> (text: String, colour: Color) {
        if let r = refunds[i.id], r.isFullyRefunded { return ("refunded", Theme.warn) }
        if let r = refunds[i.id], r.isPartlyRefunded { return ("part refunded", Theme.warn) }
        return (i.status, i.status == "succeeded" ? Theme.success : .secondary)
    }

    var body: some View {
        NavigationStack {
            List {
                Section { VStack(alignment: .leading) { Text("Collected").font(.subheadline).foregroundStyle(.secondary); Text(collected.display).font(.title.bold()) } }
                if let error { Text(error).foregroundStyle(Theme.danger) }
                ForEach(items) { i in
                    NavigationLink { PaymentDetail(intent: i) } label: {
                        HStack { VStack(alignment: .leading) { Text(i.money.display).font(.body.weight(.semibold)); Text(i.id).font(.caption2).foregroundStyle(.secondary) }
                            Spacer(); Text(badge(i).text).font(.caption.weight(.semibold)).foregroundStyle(badge(i).colour) }
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
        do {
            items = try await c.listIntents(); error = nil
            // Unfinished payments only turn paid when the server is asked, so ask for the most recent ones.
            let open = items.filter { $0.status == "processing" }.prefix(10).map(\.id)
            if !open.isEmpty {
                await withTaskGroup(of: Void.self) { g in
                    for id in open { g.addTask { _ = try? await c.settle(id: id) } }
                }
                items = try await c.listIntents()
            }
            // Refund position of the most recent paid payments, so the list can say "refunded".
            let paid = items.filter { $0.status == "succeeded" }.prefix(20).map(\.id)
            let states = await withTaskGroup(of: (String, RefundState?).self) { g -> [String: RefundState] in
                for id in paid { g.addTask { (id, try? await c.refundState(id: id)) } }
                var out: [String: RefundState] = [:]
                for await (id, s) in g { if let s { out[id] = s } }
                return out
            }
            refunds = states
        } catch { self.error = error.localizedDescription }
    }
}

struct PaymentDetail: View {
    @EnvironmentObject var store: MerchantStore
    let intent: PaymentIntent
    @State private var state: RefundState?
    @State private var message: String?
    @State private var refunding = false

    var body: some View {
        Form {
            LabeledContent("Amount", value: intent.money.display)
            LabeledContent("Status", value: statusText)
            LabeledContent("ID", value: intent.id)
            if let s = state, s.refundedMinor > 0 {
                LabeledContent("Refunded", value: Money(minor: s.refundedMinor, currency: intent.currency).display)
            }
            if intent.status == "succeeded", let s = state {
                Section {
                    if s.remainingMinor > 0 {
                        Button(s.isPartlyRefunded ? "Refund the remaining \(Money(minor: s.remainingMinor, currency: intent.currency).display)" : "Refund in full", role: .destructive) { refund(s.remainingMinor) }
                            .disabled(refunding)
                    } else {
                        Label("Fully refunded", systemImage: "arrow.uturn.backward.circle.fill").foregroundStyle(Theme.warn)
                    }
                    if let message { Text(message).font(.footnote) }
                } footer: { Text("Test mode: the refund is recorded against the simulator.") }
            }
        }
        .navigationTitle("Payment")
        .task { await reload() }
    }

    var statusText: String {
        if let s = state, s.isFullyRefunded { return "refunded" }
        if let s = state, s.isPartlyRefunded { return "part refunded" }
        return intent.status
    }

    func reload() async {
        guard let c = store.client else { return }
        state = try? await c.refundState(id: intent.id)
    }

    func refund(_ remaining: Int64) {
        guard let c = store.client else { return }
        refunding = true
        Task {
            defer { refunding = false }
            do {
                let r = try await c.refund(intentId: intent.id, amount: Money(minor: remaining, currency: intent.currency), reason: "requested_by_merchant")
                message = "Refund \(r.status)"
            } catch { message = error.localizedDescription }
            await reload()
        }
    }
}

struct MerchantSettings: View {
    @EnvironmentObject var store: MerchantStore
    @State private var scanning = false
    @State private var setupMessage: String?
    var body: some View {
        NavigationStack {
            Form {
                Section("Quick setup") {
                    Button { scanning = true } label: { Label("Scan setup QR", systemImage: "qrcode.viewfinder") }
                    if let setupMessage { Text(setupMessage).font(.footnote).foregroundStyle(.secondary) }
                    Text("Fills in the server and a test key from a QR shown on your computer. Live keys are refused.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
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
            }
            .navigationTitle("Settings")
            .fullScreenCover(isPresented: $scanning) {
                ScannerView { text in
                    scanning = false
                    do {
                        let p = try SetupPayload.parse(text)
                        store.serverURL = p.server.absoluteString
                        store.apiKey = p.apiKey
                        setupMessage = "Set up for \(p.server.host ?? "server"). Open Payments."
                    } catch SetupPayload.Failure.liveKeyRefused {
                        setupMessage = "That QR holds a live key. These apps are test-only, so it was refused."
                    } catch {
                        setupMessage = "That is not a Vayora setup QR."
                    }
                } onCancel: { scanning = false }
            }
        }
    }
}
