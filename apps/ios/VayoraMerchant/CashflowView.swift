import SwiftUI
import Charts
import VayoraKit
import VayoraUI

/// What is coming in (owed by due week), what is waiting to go out, and what the providers settled to the bank.
struct CashflowView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var flow: Cashflow?
    @State private var settlements: [SettlementInfo] = []
    @State private var message: String?
    @State private var period = GSTPeriod.containing(Date()).shifted(by: -1)   // the last full month, which is the one being filed
    @State private var gst: GSTSummary?
    @State private var gstNote: String?
    @State private var explaining: SettlementInfo?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.1)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Cashflow").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                    if let f = flow { owedCard(f); weeksCard(f) }
                    settlementsCard
                    gstCard
                    Text("A schedule of what is owed by its due date. It does not predict when people will actually pay. Rupee invoices only.").vfont(12).foregroundStyle(VUI.textTertiary)
                }.padding(20)
            }
            .refreshable { await load() }
        }
        .environment(\.colorScheme, .dark)
        .task { await load(); await loadGST() }
        .sheet(item: $explaining) { SettlementSheet(settlement: $0).environmentObject(store) }
    }

    func line(_ label: String, _ minor: Int64, hot: Bool = false) -> some View {
        HStack { Text(label).vfont(14).foregroundStyle(VUI.textSecondary); Spacer(); Text(Money(minor: minor).display).vfont(14, .semibold).foregroundStyle(hot && minor > 0 ? VUI.coral : VUI.textPrimary) }
            .accessibilityElement(children: .combine)
    }

    func owedCard(_ f: Cashflow) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 8) {
                Text("Still to come in").vfont(13).foregroundStyle(VUI.textSecondary)
                Text(Money(minor: f.totalOwedMinor).display).vfont(30, .bold).foregroundStyle(VUI.mint)
                line("Already overdue", f.overdueMinor, hot: true)
                line("Customers have promised", f.promisedMinor)
                line("Payouts waiting to go out", f.payoutsWaitingMinor)
                line("Recent receipts, per day", f.dailyReceiptsRunRateMinor)
            }
        }
    }

    func weeksCard(_ f: Cashflow) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                Label("Due by week", systemImage: "calendar").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                if f.weeks.allSatisfy({ $0.dueMinor == 0 }) {
                    Text("Nothing falls due in these weeks.").vfont(14).foregroundStyle(VUI.textSecondary)
                } else {
                    Chart(f.weeks) { w in
                        BarMark(x: .value("Week starting", String(w.starts.suffix(5))), y: .value("Due, rupees", Double(w.dueMinor) / 100)).foregroundStyle(VUI.mint)
                    }
                    .frame(height: 140)
                    .accessibilityLabel("Amount due by week: " + f.weeks.map { "week of \($0.starts), \(Money(minor: $0.dueMinor).display)" }.joined(separator: "; "))
                }
                if f.dueLaterMinor > 0 { line("Due later still", f.dueLaterMinor) }
                if f.noDueDateMinor > 0 { line("No due date", f.noDueDateMinor) }
            }
        }
    }

    /// The month's GST totals from the GSTR-1 style report. Zyvor prepares it to file from; it does not file it.
    var gstCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label("GST", systemImage: "percent").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                    Spacer()
                    Button { period = period.shifted(by: -1); Task { await loadGST() } } label: { Image(systemName: "chevron.left").frame(width: 36, height: 36) }.accessibilityLabel("Earlier month")
                    Text(period.title).vfont(14, .semibold).foregroundStyle(VUI.textPrimary)
                    Button { period = period.shifted(by: 1); Task { await loadGST() } } label: { Image(systemName: "chevron.right").frame(width: 36, height: 36) }
                        .disabled(period == GSTPeriod.containing(Date())).accessibilityLabel("Later month")
                }.foregroundStyle(VUI.mint)
                if let g = gst {
                    if g.isEmpty { Text("No sales or credit notes in \(period.title).").vfont(14).foregroundStyle(VUI.textSecondary) }
                    else {
                        line("Taxable value", g.taxableMinor)
                        line("CGST", g.cgstMinor); line("SGST", g.sgstMinor); line("IGST", g.igstMinor)
                        HStack { Text("Tax in total").vfont(14, .semibold).foregroundStyle(VUI.textPrimary); Spacer(); Text(Money(minor: g.taxMinor).display).vfont(14, .semibold).foregroundStyle(VUI.mint) }
                            .accessibilityElement(children: .combine)
                        Text("\(g.b2bInvoices) invoice\(g.b2bInvoices == 1 ? "" : "s") to registered businesses, \(g.creditNotes) credit note\(g.creditNotes == 1 ? "" : "s").").vfont(12).foregroundStyle(VUI.textTertiary)
                    }
                }
                if let gstNote { Text(gstNote).vfont(13).foregroundStyle(VUI.textSecondary) }
                Text("A summary to prepare your return from. Zyvor does not file it and the figures are not checked against the GST portal.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
        }
    }

    func loadGST() async {
        guard let c = store.client else { return }
        do { gst = try await c.gstSummary(period: period); gstNote = nil }
        catch { gst = nil; gstNote = (error as? APIError)?.message ?? "Couldn't load the GST summary." }
    }

    var settlementsCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Settled to your bank", systemImage: "building.columns").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                if settlements.isEmpty { Text("No settlements yet.").vfont(14).foregroundStyle(VUI.textSecondary) }
                ForEach(settlements.prefix(10)) { s in
                    Button { explaining = s } label: {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(Money(minor: s.netMinor).display).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            Text(s.settledOn ?? s.status).vfont(12).foregroundStyle(VUI.textSecondary)
                        }
                        Text("Gross \(Money(minor: s.grossMinor).display), \(Money(minor: s.deductionsMinor).display) taken (fees \(Money(minor: s.feeMinor).display), refunds \(Money(minor: s.refundsMinor).display))").vfont(12).foregroundStyle(VUI.textSecondary)
                        Text([s.provider, s.utr.map { "UTR \($0)" }, s.status != "matched" ? s.status : nil].compactMap { $0 }.joined(separator: " · ")).vfont(11).foregroundStyle(VUI.textTertiary)
                    }
                    }.buttonStyle(.plain).accessibilityElement(children: .combine).accessibilityHint("Explains what was taken off")
                }
            }
        }
    }

    func load() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; return }
        do {
            async let f = c.cashflow(); async let s = c.settlements()
            (flow, settlements) = try await (f, s); message = nil
        } catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
    }
}

/// Why one settlement paid what it did, in the server's own plain sentence, then the gap between what was expected and what arrived.
struct SettlementSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    let settlement: SettlementInfo
    @State private var why: SettlementExplanation?
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("This settlement").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    Text(Money(minor: settlement.netMinor).display).vfont(32, .bold).foregroundStyle(VUI.mint)
                    if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                    if let w = why {
                        GlassCard { Text(w.narrative).vfont(15).foregroundStyle(VUI.textPrimary).fixedSize(horizontal: false, vertical: true) }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 8) {
                                Label(w.verdict, systemImage: w.differenceMinor == 0 ? "checkmark.circle" : "exclamationmark.circle").vfont(16, .semibold).foregroundStyle(w.differenceMinor == 0 ? VUI.mint : VUI.amber)
                                row("Expected", w.expectedMinor); row("Received", w.receivedMinor)
                                ForEach(w.breakdown) { b in row(b.reason.prefix(1).uppercased() + b.reason.dropFirst(), b.amountMinor) }
                                if !w.balances { Text("The provider's own figures do not add up. Raise it with them.").vfont(12).foregroundStyle(VUI.coral) }
                                if w.paymentsUnmatched > 0 { Text("\(w.paymentsUnmatched) payment\(w.paymentsUnmatched == 1 ? "" : "s") here are not in your records.").vfont(12).foregroundStyle(VUI.coral) }
                                Text(w.matchedBank ? "Matched to a line on your bank statement." : "Not yet matched to a bank statement line.").vfont(12).foregroundStyle(VUI.textTertiary)
                            }
                        }
                        ForEach(w.cases) { c in
                            GlassCard { VStack(alignment: .leading, spacing: 4) {
                                Text("\(c.kind.replacingOccurrences(of: "_", with: " ").capitalized) · \(c.status)").vfont(14, .semibold).foregroundStyle(VUI.textPrimary)
                                if let e = c.explanation { Text(e).vfont(13).foregroundStyle(VUI.textSecondary) }
                            } }
                        }
                    }
                }.padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.large])
        .task {
            guard let c = store.client else { return }
            do { why = try await c.explainSettlement(id: settlement.id) } catch { message = (error as? APIError)?.message ?? "Couldn't explain this settlement." }
        }
    }

    func row(_ label: String, _ minor: Int64) -> some View {
        HStack(alignment: .top) {
            Text(label).vfont(13).foregroundStyle(VUI.textSecondary).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 12)
            Text((minor < 0 ? "−" : "") + Money(minor: abs(minor)).display).vfont(13, .semibold).foregroundStyle(VUI.textPrimary)
        }.accessibilityElement(children: .combine)
    }
}
