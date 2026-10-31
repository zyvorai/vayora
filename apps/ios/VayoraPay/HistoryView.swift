import SwiftUI
import VayoraKit

struct HistoryView: View {
    @EnvironmentObject var store: Store
    @State private var query = ""
    var rows: [Txn] {
        query.isEmpty ? store.history : store.history.filter { ($0.name + $0.payee + $0.note).localizedCaseInsensitiveContains(query) }
    }
    var body: some View {
        NavigationStack {
            List {
                if rows.isEmpty { ContentUnavailableView("No payments", systemImage: "clock", description: Text("Payments you make appear here.")) }
                ForEach(rows) { t in
                    NavigationLink { Receipt(txn: t) } label: { TxnRow(txn: t) }
                        .listRowBackground(Color.clear).listRowSeparator(.hidden)
                }
            }
            .listStyle(.plain).background(Theme.canvas)
            .searchable(text: $query, prompt: "Search payments")
            .navigationTitle("History")
        }
    }
}

struct Receipt: View {
    let txn: Txn
    var text: String {
        "Vayora Pay (TEST) receipt\n\(txn.money.display) to \(txn.name.isEmpty ? txn.payee : txn.name) (\(txn.payee))\n\(txn.status == .success ? "Paid" : "Failed") · \(txn.date.formatted())\nNo real money moved."
    }
    var body: some View {
        VStack(spacing: 14) {
            TestModeChip()
            Image(systemName: txn.status == .success ? "checkmark.seal.fill" : "xmark.seal.fill").font(.system(size: 64))
                .foregroundStyle(txn.status == .success ? Theme.success : Theme.danger)
            Text(txn.money.display).font(.largeTitle.bold())
            Text(txn.status == .success ? "Paid" : "Failed").foregroundStyle(.secondary)
            Card {
                VStack(alignment: .leading, spacing: 6) {
                    row("To", txn.name.isEmpty ? txn.payee : txn.name); row("UPI ID", txn.payee)
                    row("Reference", txn.note); row("Date", txn.date.formatted())
                }
            }
            ShareLink(item: text) { Label("Share receipt", systemImage: "square.and.arrow.up") }.buttonStyle(PrimaryButtonStyle())
            Spacer()
        }
        .padding(20).background(Theme.canvas).navigationTitle("Receipt").navigationBarTitleDisplayMode(.inline)
    }
    func row(_ k: String, _ v: String) -> some View {
        HStack { Text(k).foregroundStyle(.secondary); Spacer(); Text(v).multilineTextAlignment(.trailing) }.font(.subheadline)
    }
}
