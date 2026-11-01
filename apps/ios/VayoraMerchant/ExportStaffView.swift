import SwiftUI
import VayoraKit
import VayoraUI

/// Files an accountant asks for: every payment and a total per day (Indian days), with refunds taken off.
struct AccountantExportView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var payments: URL?
    @State private var daily: URL?
    @State private var message: String? = "Preparing…"
    @State private var capped = false

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.45)
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text("Accountant export").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        if let payments { ShareLink(item: payments) { Label("Every payment (CSV)", systemImage: "tablecells") }.vfont(16, .semibold).foregroundStyle(VUI.mint) }
                        if let daily { ShareLink(item: daily) { Label("Totals per day (CSV)", systemImage: "calendar") }.vfont(16, .semibold).foregroundStyle(VUI.mint) }
                        if let message { Text(message).vfont(13).foregroundStyle(VUI.textSecondary) }
                        Text("Days are Indian days (IST). Totals count paid payments only, net of refunds. Test mode: this is not a tax document.").vfont(12).foregroundStyle(VUI.textTertiary)
                    }
                }
                Spacer()
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
        .task { await build() }
    }

    func build() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; return }
        do {
            let items = try await c.listIntents()
            let iso = ISO8601DateFormatter(), frac = ISO8601DateFormatter.withFraction
            let slice = Array(items.prefix(300)); capped = items.count > 300
            // the refund position of each paid payment, a few at a time
            let refunds = await withTaskGroup(of: (String, Int64).self) { g -> [String: Int64] in
                for i in slice where i.status == "succeeded" { g.addTask { (i.id, (try? await c.refundState(id: i.id))??.refundedMinor ?? 0) } }
                var out: [String: Int64] = [:]; for await (id, m) in g { out[id] = m }; return out
            }
            let rows: [PaymentCSVRow] = slice.compactMap { i in
                guard let s = i.createdAt, let d = iso.date(from: s) ?? frac.date(from: s) else { return nil }
                return PaymentCSVRow(date: d, id: i.id, amountMinor: i.amountMinor, status: i.status, refundedMinor: refunds[i.id] ?? 0)
            }
            let dir = FileManager.default.temporaryDirectory
            let p = dir.appendingPathComponent("vayora-payments.csv"), d = dir.appendingPathComponent("vayora-daily-totals.csv")
            try PaymentsCSV.payments(rows).write(to: p, atomically: true, encoding: .utf8)
            try PaymentsCSV.daily(rows).write(to: d, atomically: true, encoding: .utf8)
            payments = p; daily = d
            message = capped ? "The most recent 300 payments are included." : "\(rows.count) payments."
        } catch { message = (error as? APIError)?.message ?? "Couldn't prepare the files." }
    }
}

/// Staff names for this till, and today's takings by name.
struct StaffView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var newName = ""
    @State private var totals: [RegularsBook.Total] = []
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.35)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack { Text("Staff").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                    GlassCard {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(store.staff, id: \.self) { n in
                                HStack { Text(n).foregroundStyle(VUI.textPrimary); Spacer()
                                    Button { store.staff.removeAll { $0 == n }; if store.activeStaff == n { store.activeStaff = nil } } label: { Image(systemName: "minus.circle").foregroundStyle(VUI.textTertiary) }.accessibilityLabel("Remove \(n)") }.vfont(16)
                            }
                            HStack { TextField("Add a name", text: $newName).foregroundStyle(VUI.textPrimary).padding(10).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                                Button("Add") { add() }.vfont(15, .semibold).foregroundStyle(VUI.mint).disabled(RegularsBook.clean(newName).isEmpty) }
                            Text("Pick who is on the till on the Collect screen. Each sale is tagged with that name on this phone.").vfont(12).foregroundStyle(VUI.textTertiary)
                        }
                    }
                    GlassCard {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Today").vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                            if totals.isEmpty { Text("No tagged sales today.").vfont(14).foregroundStyle(VUI.textSecondary) }
                            ForEach(totals, id: \.name) { t in HStack { Text("\(t.name) (\(t.count))").foregroundStyle(VUI.textSecondary); Spacer(); Text(Money(minor: t.minor).display).foregroundStyle(VUI.mint) }.vfont(15) }
                            if let message { Text(message).vfont(13).foregroundStyle(VUI.textSecondary) }
                            Text("Paid sales made on this phone, today (IST). Refunds are not subtracted.").vfont(12).foregroundStyle(VUI.textTertiary)
                        }
                    }
                }.padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .task { await load() }
    }

    func add() {
        let n = String(RegularsBook.clean(newName).prefix(24))
        if !n.isEmpty, !store.staff.contains(where: { $0.lowercased() == n.lowercased() }), store.staff.count < 12 { store.staff.append(n) }
        newName = ""
    }

    func load() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; return }
        do {
            let items = try await c.listIntents()
            let iso = ISO8601DateFormatter(), frac = ISO8601DateFormatter.withFraction, cal = PaymentsCSV.ist
            let today = items.filter { i in
                guard let s = i.createdAt, let d = iso.date(from: s) ?? frac.date(from: s) else { return false }
                return cal.isDate(d, inSameDayAs: Date())
            }
            totals = store.staffTags.totals(today.map { ($0.id, $0.amountMinor, $0.status == "succeeded") })
        } catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
    }
}
