import SwiftUI
import VayoraKit
import VayoraUI

struct HomeView: View {
    @EnvironmentObject var store: Store
    @State private var scanning = false
    @State private var pasting = false
    @State private var payload: UPIPayload?
    @State private var scanError: String?
    @State private var receipt: Txn?

    var body: some View {
        NavigationStack {
            PayHomeScreen(balance: store.balance, spentThisMonth: Money(minor: store.spentThisMonth), week: weekSeries, recent: store.history.prefix(4).map(row),
                          onScan: { scanning = true }, onPaste: { pasting = true },
                          onOpen: { r in receipt = store.history.first { $0.id.uuidString == r.id } })
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(item: $receipt) { Receipt(txn: $0) }
                .fullScreenCover(isPresented: $scanning) {
                    ScannerView(brand: .pay) { text in
                        scanning = false
                        open(text)
                    } onCancel: { scanning = false }
                }
                .sheet(isPresented: $pasting) { PasteSheet { open($0) } }
                .sheet(item: Binding(get: { payload.map(Box.init) }, set: { if $0 == nil { payload = nil } })) { b in
                    PaySheet(payload: b.value).environmentObject(store)
                }
                .alert("Can't read that code", isPresented: Binding(get: { scanError != nil }, set: { _ in scanError = nil })) {
                    Button("OK", role: .cancel) {}
                } message: { Text(scanError ?? "") }
        }
    }

    struct Box: Identifiable { let value: UPIPayload; var id: String { value.payee + value.note } }

    func open(_ text: String) {
        do { payload = try UPIPayload.parse(text) } catch { scanError = "That is not a UPI payment code." }
    }

    func row(_ t: Txn) -> TxnRowModel {
        TxnRowModel(id: t.id.uuidString, name: t.name, payee: t.payee, amount: t.money,
                    when: t.date.formatted(.relative(presentation: .named)), ok: t.status == .success)
    }

    /// Spend per day for the last seven days, oldest first.
    var weekSeries: [Double] {
        let cal = Calendar.current, today = cal.startOfDay(for: Date())
        return (0..<7).reversed().map { back in
            let day = cal.date(byAdding: .day, value: -back, to: today)!
            return Double(store.history.filter { $0.status == .success && cal.isDate($0.date, inSameDayAs: day) }.reduce(Int64(0)) { $0 + $1.amountMinor }) / 100
        }
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
