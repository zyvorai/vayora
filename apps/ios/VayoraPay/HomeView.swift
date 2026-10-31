import SwiftUI
import VayoraKit

struct HomeView: View {
    @EnvironmentObject var store: Store
    @State private var scanning = false
    @State private var pasting = false
    @State private var payload: UPIPayload?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 16) {
                    TestModeChip()
                    balanceCard
                    HStack(spacing: 12) {
                        action("Scan & pay", "qrcode.viewfinder") { scanning = true }
                        action("Paste UPI link", "link") { pasting = true }
                    }
                    insights
                    recent
                }
                .padding(16)
            }
            .background(Theme.canvas)
            .navigationTitle("Vayora Pay")
            .fullScreenCover(isPresented: $scanning) {
                ScannerView { text in
                    scanning = false
                    open(text)
                } onCancel: { scanning = false }
            }
            .sheet(isPresented: $pasting) { PasteSheet { open($0) } }
            .sheet(item: Binding(get: { payload.map(Box.init) }, set: { if $0 == nil { payload = nil } })) { b in
                PaySheet(payload: b.value).environmentObject(store)
            }
        }
    }

    struct Box: Identifiable { let value: UPIPayload; var id: String { value.payee + value.note } }
    @State private var scanError: String?

    func open(_ text: String) {
        do { payload = try UPIPayload.parse(text) } catch { scanError = "That is not a UPI payment code." }
    }

    var balanceCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Test balance").font(.subheadline).foregroundStyle(.white.opacity(0.8))
            Text(store.balance.display).font(.system(size: 40, weight: .bold, design: .rounded)).foregroundStyle(.white)
                .contentTransition(.numericText())
            Text("Held on this phone, not in a bank").font(.caption).foregroundStyle(.white.opacity(0.75))
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(20)
        .background(Theme.heroGradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
        .accessibilityElement(children: .combine)
    }

    func action(_ title: String, _ icon: String, _ run: @escaping () -> Void) -> some View {
        Button(action: run) {
            VStack(spacing: 8) {
                Image(systemName: icon).font(.title2)
                Text(title).font(.subheadline.weight(.semibold))
            }
            .frame(maxWidth: .infinity).padding(.vertical, 18)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
        .buttonStyle(.plain)
    }

    var insights: some View {
        Card {
            VStack(alignment: .leading, spacing: 4) {
                Text("Spent this month").font(.subheadline).foregroundStyle(.secondary)
                Text(Money(minor: store.spentThisMonth).display).font(.title2.bold())
            }
        }
    }

    var recent: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Recent").font(.headline)
            if store.history.isEmpty {
                Card { Text("No payments yet. Scan a Vayora Merchant QR to make one.").foregroundStyle(.secondary) }
            } else {
                ForEach(store.history.prefix(4)) { TxnRow(txn: $0) }
            }
        }
        .alert("Can't read that code", isPresented: Binding(get: { scanError != nil }, set: { _ in scanError = nil })) {
            Button("OK", role: .cancel) {}
        } message: { Text(scanError ?? "") }
    }
}

struct TxnRow: View {
    let txn: Txn
    var body: some View {
        Card {
            HStack {
                Image(systemName: txn.status == .success ? "checkmark.circle.fill" : "xmark.circle.fill")
                    .foregroundStyle(txn.status == .success ? Theme.success : Theme.danger).font(.title2)
                VStack(alignment: .leading) {
                    Text(txn.name.isEmpty ? txn.payee : txn.name).font(.body.weight(.semibold))
                    Text(txn.date, style: .relative).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Text(txn.money.display).font(.body.weight(.semibold))
            }
        }
        .accessibilityElement(children: .combine)
    }
}

struct PasteSheet: View {
    @Environment(\.dismiss) var dismiss
    @State private var text = ""
    var onOpen: (String) -> Void
    var body: some View {
        NavigationStack {
            Form {
                Section("upi://pay link") { TextField("upi://pay?pa=…", text: $text, axis: .vertical).textInputAutocapitalization(.never).autocorrectionDisabled() }
            }
            .navigationTitle("Paste UPI link")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("Continue") { dismiss(); onOpen(text) }.disabled(text.isEmpty) }
            }
        }
    }
}
