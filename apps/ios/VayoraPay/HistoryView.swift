import SwiftUI
import VayoraKit
import VayoraUI

struct HistoryView: View {
    @EnvironmentObject var store: Store
    @State private var query = ""
    @State private var open: Txn?

    var rows: [Txn] {
        query.isEmpty ? store.history : store.history.filter { ($0.name + $0.payee + $0.note).localizedCaseInsensitiveContains(query) }
    }
    /// Newest day first, each day's payments newest first.
    var days: [(day: Date, items: [Txn])] {
        let cal = Calendar.current
        return Dictionary(grouping: rows) { cal.startOfDay(for: $0.date) }
            .map { (day: $0.key, items: $0.value.sorted { $0.date > $1.date }) }.sorted { $0.day > $1.day }
    }
    func title(_ d: Date) -> String {
        Calendar.current.isDateInToday(d) ? "Today" : Calendar.current.isDateInYesterday(d) ? "Yesterday" : d.formatted(.dateTime.weekday(.wide).day().month())
    }

    var body: some View {
        NavigationStack {
            ZStack {
                AmbientBackground(brand: .pay, phase: 0.35)
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        Text("History").font(VUI.display(28)).foregroundStyle(VUI.textPrimary)
                        HStack(spacing: 8) {
                            Image(systemName: "magnifyingglass").foregroundStyle(VUI.textTertiary)
                            TextField("Search payments", text: $query).textInputAutocapitalization(.never).autocorrectionDisabled().foregroundStyle(VUI.textPrimary)
                        }
                        .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                        if rows.isEmpty {
                            GlassCard { Text(query.isEmpty ? "Payments you make appear here." : "Nothing matches “\(query)”.").font(.system(size: 15, design: .rounded)).foregroundStyle(VUI.textSecondary) }
                        }
                        ForEach(days, id: \.day) { group in
                            Text(title(group.day).uppercased()).font(.system(size: 12, weight: .bold, design: .rounded)).kerning(0.8).foregroundStyle(VUI.textTertiary).padding(.top, 6)
                            ForEach(group.items) { t in
                                Button { open = t } label: {
                                    TxnRowView(model: TxnRowModel(id: t.id.uuidString, name: t.name, payee: t.payee, amount: t.money, when: t.date.formatted(date: .omitted, time: .shortened), ok: t.status == .success))
                                }.buttonStyle(PressScale())
                            }
                        }
                    }
                    .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
                }
            }
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(item: $open) { Receipt(txn: $0) }
        }
    }
}

struct Receipt: View {
    let txn: Txn
    @State private var progress = 0.0
    var ok: Bool { txn.status == .success }
    var who: String { txn.name.isEmpty ? txn.payee : txn.name }
    var text: String {
        "Vayora Pay (TEST) receipt\n\(txn.money.display) to \(who) (\(txn.payee))\n\(ok ? "Paid" : "Failed") · \(txn.date.formatted())\nNo real money moved."
    }
    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.45)
            ScrollView {
                VStack(spacing: 18) {
                    TestChip()
                    if ok { CheckmarkDraw(progress: progress).frame(width: 96, height: 96) }
                    else { Image(systemName: "xmark.circle.fill").font(.system(size: 64)).foregroundStyle(VUI.coral) }
                    AmountText(txn.money, size: 56)
                    StatusPill(ok ? "Paid" : "Failed", ok ? .success : .danger)
                    GlassCard {
                        VStack(alignment: .leading, spacing: 12) {
                            row("To", who); row("UPI ID", txn.payee); row("Reference", txn.note); row("Date", txn.date.formatted(date: .abbreviated, time: .shortened))
                        }
                    }
                    ShareLink(item: text) {
                        Label("Share receipt", systemImage: "square.and.arrow.up").font(VUI.display(17, .semibold)).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 16)
                            .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                    }.buttonStyle(PressScale())
                    Text("Test receipt. No real money moved.").font(.system(size: 12, design: .rounded)).foregroundStyle(VUI.textTertiary)
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .navigationTitle("Receipt").navigationBarTitleDisplayMode(.inline)
        .onAppear { withAnimation(.easeOut(duration: 0.9)) { progress = 1 } }
    }
    func row(_ k: String, _ v: String) -> some View {
        HStack(alignment: .firstTextBaseline) { Text(k).foregroundStyle(VUI.textSecondary); Spacer(); Text(v).foregroundStyle(VUI.textPrimary).multilineTextAlignment(.trailing) }.font(.system(size: 15, design: .rounded))
    }
}
