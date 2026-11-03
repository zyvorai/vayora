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
                    Text("A schedule of what is owed by its due date. It does not predict when people will actually pay. Rupee invoices only.").vfont(12).foregroundStyle(VUI.textTertiary)
                }.padding(20)
            }
            .refreshable { await load() }
        }
        .environment(\.colorScheme, .dark)
        .task { await load() }
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

    var settlementsCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Settled to your bank", systemImage: "building.columns").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                if settlements.isEmpty { Text("No settlements yet.").vfont(14).foregroundStyle(VUI.textSecondary) }
                ForEach(settlements.prefix(10)) { s in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Text(Money(minor: s.netMinor).display).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            Text(s.settledOn ?? s.status).vfont(12).foregroundStyle(VUI.textSecondary)
                        }
                        Text("Gross \(Money(minor: s.grossMinor).display), \(Money(minor: s.deductionsMinor).display) taken (fees \(Money(minor: s.feeMinor).display), refunds \(Money(minor: s.refundsMinor).display))").vfont(12).foregroundStyle(VUI.textSecondary)
                        Text([s.provider, s.utr.map { "UTR \($0)" }, s.status != "matched" ? s.status : nil].compactMap { $0 }.joined(separator: " · ")).vfont(11).foregroundStyle(VUI.textTertiary)
                    }
                    .accessibilityElement(children: .combine)
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
