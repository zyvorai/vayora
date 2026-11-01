import SwiftUI
import VayoraKit
import VayoraUI

struct HistoryView: View {
    @EnvironmentObject var store: Store
    @State private var filter = HistoryFilter()
    @State private var open: Txn?
    @State private var exportURL: URL?
    @State private var reportURL: URL?

    var query: String { filter.text }
    /// A question like "food last week" or "more than 500 this month": answered with a total above the list.
    var asked: SpendQuery? {
        let q = SpendQuery.parse(filter.text)
        return q.isStructured ? q : nil
    }
    func matching() -> [HistoryRow] {
        if var q = asked {
            var base = filter; base.text = ""
            // chips still apply on top of the question
            if let c = base.category, q.filter.category == nil { q.filter.category = c }
            if base.outcome != .all { q.filter.outcome = base.outcome }
            return q.apply(store.historyRows)
        }
        return filter.apply(store.historyRows)
    }
    var rows: [Txn] {
        let keep = Set(matching().map { RowKey($0) })
        return store.history.filter { keep.contains(RowKey($0)) }
    }
    struct RowKey: Hashable {
        let date: Date, payee: String, amount: Int64
        init(_ r: HistoryRow) { date = r.date; payee = r.payee; amount = r.amountMinor }
        init(_ t: Txn) { date = t.date; payee = t.payee; amount = t.amountMinor }
    }

    /// The CSV of what is on screen, written to a temporary file so Files, Mail or a spreadsheet app can take it.
    func export() {
        let csv = HistoryCSV.make(matching())
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("vayora-pay-history.csv")
        do { try csv.write(to: url, atomically: true, encoding: .utf8); exportURL = url } catch { exportURL = nil }
    }
    /// Newest day first, each day's payments newest first.
    var days: [(day: Date, items: [Txn])] {
        let cal = Calendar.current
        return Dictionary(grouping: rows) { cal.startOfDay(for: $0.date) }
            .map { (day: $0.key, items: $0.value.sorted { $0.date > $1.date }) }.sorted { $0.day > $1.day }
    }
    func answerCard(_ q: SpendQuery) -> some View {
        let t = SpendQuery.total(matching())
        return GlassCard {
            VStack(alignment: .leading, spacing: 4) {
                Text([q.filter.category?.title, q.periodLabel].compactMap { $0 }.joined(separator: " · ").isEmpty ? "Answer" : [q.filter.category?.title, q.periodLabel].compactMap { $0 }.joined(separator: " · "))
                    .vfont(13, .medium).foregroundStyle(VUI.textSecondary)
                AmountText(Money(minor: t.minor), size: 34)
                Text("\(t.count) payment\(t.count == 1 ? "" : "s"). Failed payments are not counted.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
        }
        .accessibilityElement(children: .combine)
    }

    var filterChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("All", filter.category == nil && filter.outcome == .all) { filter.category = nil; filter.outcome = .all }
                chip("Failed", filter.outcome == .failed) { filter.outcome = filter.outcome == .failed ? .all : .failed }
                ForEach(SpendCategory.allCases, id: \.self) { c in chip(c.title, filter.category == c) { filter.category = filter.category == c ? nil : c } }
            }
        }
        .onChange(of: filter) { _, _ in exportURL = nil }
    }
    func chip(_ t: String, _ on: Bool, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(t).vfont(13, .semibold).foregroundStyle(on ? .white : VUI.textSecondary).padding(.horizontal, 14).padding(.vertical, 8)
                .background(on ? AnyShapeStyle(VUI.violet.opacity(0.9)) : AnyShapeStyle(.white.opacity(0.08)), in: Capsule())
        }.buttonStyle(PressScale()).accessibilityAddTraits(on ? .isSelected : [])
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
                        HStack {
                            Text("History").vfont(28, .bold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            if let report = reportURL {
                                ShareLink(item: report) { Label("PDF", systemImage: "doc.richtext").vfont(14, .semibold).foregroundStyle(VUI.violet) }
                            } else if !rows.isEmpty {
                                Button { let r = MonthReport.make(store.historyRows, month: Date()); reportURL = MonthPDF.make(r) } label: { Label("Month PDF", systemImage: "doc.richtext").vfont(14, .semibold).foregroundStyle(VUI.violet) }
                            }
                            if let exportURL {
                                ShareLink(item: exportURL) { Label("Share CSV", systemImage: "square.and.arrow.up").vfont(14, .semibold).foregroundStyle(VUI.violet) }
                            } else if !rows.isEmpty {
                                Button { export() } label: { Label("Export", systemImage: "tray.and.arrow.up").vfont(14, .semibold).foregroundStyle(VUI.violet) }
                            }
                        }
                        HStack(spacing: 8) {
                            Image(systemName: "magnifyingglass").foregroundStyle(VUI.textTertiary)
                            TextField("Search, or ask: food last week", text: $filter.text).textInputAutocapitalization(.never).autocorrectionDisabled().foregroundStyle(VUI.textPrimary)
                        }
                        .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                        filterChips
                        if let q = asked { answerCard(q) }
                        if rows.isEmpty {
                            GlassCard { Text(!filter.isActive ? "Payments you make appear here." : "Nothing matches these filters.").vfont(15).foregroundStyle(VUI.textSecondary) }
                        }
                        ForEach(days, id: \.day) { group in
                            Text(title(group.day).uppercased()).vfont(12, .bold).kerning(0.8).foregroundStyle(VUI.textTertiary).padding(.top, 6)
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
    @EnvironmentObject var store: Store
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
                    VaultCard(id: txn.id.uuidString)
                    if !txn.isAutoPay {
                        GlassCard {
                            HStack {
                                Label("Category", systemImage: "tag.fill").vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                                Spacer()
                                Picker("Category", selection: Binding(get: { SpendCategory(rawValue: store.history.first { $0.id == txn.id }?.category ?? txn.category ?? "") ?? .other },
                                                                      set: { store.setCategory($0, forPayee: txn.payee) })) {
                                    ForEach(SpendCategory.allCases, id: \.self) { Text($0.title).tag($0) }
                                }.tint(VUI.violet)
                            }
                            Text("Changing it also changes your other payments to this UPI ID, and later ones.").vfont(12).foregroundStyle(VUI.textTertiary)
                        }
                    }
                    ShareLink(item: text) {
                        Label("Share receipt", systemImage: "square.and.arrow.up").vfont(17, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 16)
                            .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                    }.buttonStyle(PressScale())
                    Text("Test receipt. No real money moved.").vfont(12).foregroundStyle(VUI.textTertiary)
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .navigationTitle("Receipt").navigationBarTitleDisplayMode(.inline)
        .onAppear { withAnimation(.easeOut(duration: 0.9)) { progress = 1 } }
    }
    func row(_ k: String, _ v: String) -> some View {
        HStack(alignment: .firstTextBaseline) { Text(k).foregroundStyle(VUI.textSecondary); Spacer(); Text(v).foregroundStyle(VUI.textPrimary).multilineTextAlignment(.trailing) }.vfont(15)
    }
}
