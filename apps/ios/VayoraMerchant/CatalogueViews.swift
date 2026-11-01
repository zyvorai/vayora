import SwiftUI
import VayoraKit
import VayoraUI

/// The things this shop sells. Prices and GST are typed by the owner; nothing here is looked up or assumed.
struct CatalogueView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var name = ""
    @State private var price = ""
    @State private var gst = -1 // -1 means no GST line
    @State private var inclusive = false
    @State private var problem: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.45)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack { Text("Items").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 44, height: 44) }.accessibilityLabel("Close") }
                    ForEach(store.catalogue.products) { p in
                        GlassCard {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(p.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                                    Text(detail(p)).vfont(13).foregroundStyle(VUI.textSecondary)
                                }
                                Spacer()
                                Button(role: .destructive) { store.catalogue.remove(p.id) } label: { Image(systemName: "trash").frame(width: 44, height: 44) }.accessibilityLabel("Delete \(p.name)")
                            }
                        }
                    }
                    GlassCard {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("Add an item").vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                            TextField("Name", text: $name).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous)).foregroundStyle(VUI.textPrimary)
                            TextField("Price in rupees, e.g. 25.50", text: $price).keyboardType(.decimalPad).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous)).foregroundStyle(VUI.textPrimary)
                            Picker("GST", selection: $gst) {
                                Text("No GST line").tag(-1); Text("0%").tag(0); Text("5%").tag(5); Text("12%").tag(12); Text("18%").tag(18); Text("28%").tag(28)
                            }.tint(VUI.mint)
                            if gst > 0 { Toggle("Price already includes GST", isOn: $inclusive).tint(VUI.mint).foregroundStyle(VUI.textPrimary) }
                            if let problem { Text(problem).vfont(13).foregroundStyle(VUI.coral) }
                            Button { add() } label: { Label("Add", systemImage: "plus.circle.fill") }.vfont(16, .semibold).foregroundStyle(VUI.mint)
                            Text("GST lines are arithmetic on the numbers you type. A bill here is not a tax invoice.").vfont(12).foregroundStyle(VUI.textTertiary)
                        }
                    }
                }.padding(24)
            }
        }
        .environment(\.colorScheme, .dark)
    }

    func detail(_ p: Product) -> String {
        var s = Money(minor: p.priceMinor).display
        if let g = p.gstPercent, g > 0 { s += p.gstInclusive ? " incl. \(g)% GST" : " + \(g)% GST" }
        return s
    }

    func add() {
        guard let m = Money(decimal: price) else { problem = "Enter the price like 25 or 25.50."; return }
        do {
            try store.catalogue.add(Product(name: name, priceMinor: m.minor, gstPercent: gst < 0 ? nil : gst, gstInclusive: gst > 0 && inclusive))
            name = ""; price = ""; problem = nil
        } catch Catalogue.Failure.emptyName { problem = "Give the item a name." }
        catch Catalogue.Failure.nameTooLong { problem = "Keep the name under \(Catalogue.maxNameLength) letters." }
        catch Catalogue.Failure.duplicate { problem = "You already have an item with that name." }
        catch Catalogue.Failure.badPrice { problem = "The price must be above zero." }
        catch { problem = "Couldn't add that item." }
    }
}

/// Pick items, see the total with any GST lines, and use it as the amount to collect.
struct BillSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @Binding var bill: Bill
    var onUse: () -> Void

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.5)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack { Text("Add items").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 44, height: 44) }.accessibilityLabel("Close") }
                    if store.catalogue.products.isEmpty {
                        Text("No items yet. Add some in Settings, Items.").vfont(14).foregroundStyle(VUI.textSecondary)
                    }
                    ForEach(store.catalogue.products) { p in
                        GlassCard {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(p.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                                    Text(Money(minor: p.priceMinor).display).vfont(13).foregroundStyle(VUI.textSecondary)
                                }
                                Spacer()
                                let q = bill.quantity(of: p)
                                Button { bill.remove(p) } label: { Image(systemName: "minus.circle.fill").font(.system(size: 26)).frame(width: 44, height: 44) }.disabled(q == 0).accessibilityLabel("Remove one \(p.name)")
                                Text("\(q)").vfont(17, .semibold).foregroundStyle(VUI.textPrimary).frame(minWidth: 28)
                                Button { bill.add(p) } label: { Image(systemName: "plus.circle.fill").font(.system(size: 26)).frame(width: 44, height: 44) }.accessibilityLabel("Add one \(p.name)")
                            }.foregroundStyle(VUI.mint)
                        }
                    }
                    if !bill.isEmpty {
                        GlassCard {
                            VStack(alignment: .leading, spacing: 6) {
                                row("Items", Money(minor: bill.netMinor).display)
                                if bill.gstMinor > 0 { row("GST", Money(minor: bill.gstMinor).display) }
                                row("Total", Money(minor: bill.totalMinor).display, bold: true)
                                Text("Not a tax invoice.").vfont(12).foregroundStyle(VUI.textTertiary)
                            }
                        }
                        Button { onUse(); dismiss() } label: {
                            Text("Use \(Money(minor: bill.totalMinor).display)").vfont(17, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 16)
                                .background(LinearGradient(colors: [VUI.mint, VUI.emerald], startPoint: .top, endPoint: .bottom), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                        }.buttonStyle(PressScale())
                        Button("Clear") { bill.clear() }.vfont(14).foregroundStyle(VUI.textSecondary)
                    }
                }.padding(24)
            }
        }
        .environment(\.colorScheme, .dark)
    }

    func row(_ k: String, _ v: String, bold: Bool = false) -> some View {
        HStack { Text(k).foregroundStyle(VUI.textSecondary); Spacer(); Text(v).foregroundStyle(VUI.textPrimary) }.vfont(bold ? 17 : 15, bold ? .semibold : .regular)
    }
}

/// What sold, from bills made in this app. Items sold some other way are not here, and the screen says so.
struct SalesByItemView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var days = 7
    @State private var rows: [SalesByItem.Row] = []
    @State private var loading = true

    var title: String { "Sales by item, last \(days) days" }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack { Text("Sales by item").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 44, height: 44) }.accessibilityLabel("Close") }
                    Picker("Period", selection: $days) { Text("7 days").tag(7); Text("30 days").tag(30) }.pickerStyle(.segmented)
                    if loading { ProgressView() }
                    else if rows.isEmpty { Text("No itemised sales yet.").vfont(14).foregroundStyle(VUI.textSecondary) }
                    ForEach(rows, id: \.name) { r in
                        GlassCard { HStack { Text("\(r.quantity) x \(r.name)").foregroundStyle(VUI.textPrimary); Spacer(); Text(Money(minor: r.grossMinor).display).foregroundStyle(VUI.textSecondary) }.vfont(15) }
                    }
                    if !rows.isEmpty { ShareLink(item: SalesByItem.summary(rows, title: title)) { Label("Share", systemImage: "square.and.arrow.up") }.vfont(15, .semibold).foregroundStyle(VUI.mint) }
                    Text("Only items sold through bills made on this phone are counted. Paid payments only. Not a tax invoice.").vfont(12).foregroundStyle(VUI.textTertiary)
                }.padding(24)
            }
        }
        .environment(\.colorScheme, .dark)
        .task(id: days) { await load() }
    }

    func load() async {
        loading = true
        var paid = Set<String>()
        if let c = store.client, let list = try? await c.listIntents() { paid = Set(list.filter { $0.status == "succeeded" }.map(\.id)) }
        rows = SalesByItem.top(store.billLog, paid: paid, since: Date().addingTimeInterval(-Double(days) * 86400))
        loading = false
    }
}
