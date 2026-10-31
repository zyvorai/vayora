import SwiftUI
import VayoraKit

public extension SpendCategory {
    var colour: Color {
        switch self { case .food: VUI.amber; case .shopping: VUI.violet; case .bills: VUI.coral; case .travel: VUI.mint; case .transfers: VUI.indigo; case .other: VUI.textSecondary }
    }
    var symbol: String {
        switch self { case .food: "fork.knife"; case .shopping: "bag.fill"; case .bills: "doc.text.fill"; case .travel: "tram.fill"; case .transfers: "arrow.left.arrow.right"; case .other: "ellipsis" }
    }
}

/// This month by category: one bar each, drawn by hand so it renders the same in the Mac gallery.
public struct MonthSummaryCard: View {
    let items: [(category: SpendCategory, minor: Int64)], total: Money
    public init(items: [(category: SpendCategory, minor: Int64)], total: Money) { self.items = items; self.total = total }
    public var body: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    Text("This month").vfont(14, .medium).foregroundStyle(VUI.textSecondary)
                    Spacer()
                    Text(total.display).vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                }
                if items.isEmpty { Text("Paid payments show up here by category.").vfont(14).foregroundStyle(VUI.textTertiary) }
                let top = max(1, items.first?.minor ?? 1)
                ForEach(items, id: \.category) { it in
                    VStack(alignment: .leading, spacing: 5) {
                        HStack {
                            Label(it.category.title, systemImage: it.category.symbol).vfont(13, .semibold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            Text(Money(minor: it.minor).display).vfont(13).foregroundStyle(VUI.textSecondary)
                        }
                        GeometryReader { g in
                            Capsule().fill(it.category.colour.opacity(0.9)).frame(width: max(8, g.size.width * CGFloat(it.minor) / CGFloat(top)))
                        }.frame(height: 7).background(Capsule().fill(.white.opacity(0.08)))
                    }
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("This month \(total.display). " + items.map { "\($0.category.title) \(Money(minor: $0.minor).display)" }.joined(separator: ", "))
    }
}

public struct PayeeRowView: View {
    let payee: Payee, onStar: () -> Void
    public init(payee: Payee, onStar: @escaping () -> Void = {}) { self.payee = payee; self.onStar = onStar }
    public var body: some View {
        HStack(spacing: 14) {
            AvatarBadge(seed: payee.vpa, label: payee.name, size: 46)
            VStack(alignment: .leading, spacing: 3) {
                Text(payee.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary).lineLimit(1)
                Text(payee.vpa).vfont(12.5).foregroundStyle(VUI.textTertiary).lineLimit(1).truncationMode(.middle)
            }
            Spacer()
            Button(action: { Haptics.tap(); onStar() }) {
                Image(systemName: payee.favourite ? "star.fill" : "star").font(.system(size: 18, weight: .semibold)).foregroundStyle(payee.favourite ? VUI.amber : VUI.textTertiary)
                    .frame(width: 44, height: 44)
            }.buttonStyle(.plain).accessibilityLabel(payee.favourite ? "Remove from favourites" : "Add to favourites")
        }
        .padding(.leading, 14).padding(.vertical, 8).padding(.trailing, 4)
        .background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.hairline))
    }
}

/// Everyone you pay: favourites first, then the most recent.
public struct PayeesScreen: View {
    let payees: [Payee], scrolls: Bool, onOpen: (Payee) -> Void, onStar: (Payee) -> Void, onAdd: () -> Void
    public init(payees: [Payee], scrolls: Bool = true, onOpen: @escaping (Payee) -> Void = { _ in }, onStar: @escaping (Payee) -> Void = { _ in }, onAdd: @escaping () -> Void = {}) {
        self.payees = payees; self.scrolls = scrolls; self.onOpen = onOpen; self.onStar = onStar; self.onAdd = onAdd
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.62)
            if scrolls { ScrollView { content } } else { content.frame(maxHeight: .infinity, alignment: .top) }
        }
        .environment(\.colorScheme, .dark)
    }
    var content: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Payees").vfont(28, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Spacer()
                Button(action: onAdd) {
                    Label("Add", systemImage: "plus").vfont(15, .semibold).foregroundStyle(.white).padding(.horizontal, 14).padding(.vertical, 9)
                        .background(Brand.pay.gradient, in: Capsule())
                }.buttonStyle(PressScale())
            }
            if payees.isEmpty {
                GlassCard { Text("Nobody yet. Add a UPI ID or phone number, or pay someone and they appear here.").vfont(15).foregroundStyle(VUI.textSecondary) }
            }
            ForEach(payees) { p in
                Button { onOpen(p) } label: { PayeeRowView(payee: p, onStar: { onStar(p) }) }.buttonStyle(PressScale())
            }
            Text("Test transfers to people are recorded on this phone only. They are not in Vayora's ledger.").vfont(12).foregroundStyle(VUI.textTertiary).padding(.top, 6)
        }
        .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
    }
}

/// The request QR for one person's share of a bill.
public struct SplitShareScreen: View {
    let total: Money, people: Int, share: Money, qr: Image?, myName: String, onDone: () -> Void
    public init(total: Money, people: Int, share: Money, qr: Image?, myName: String, onDone: @escaping () -> Void = {}) {
        self.total = total; self.people = people; self.share = share; self.qr = qr; self.myName = myName; self.onDone = onDone
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.7)
            VStack(spacing: 18) {
                HStack { Text("Split").vfont(28, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
                Text("\(total.display) between \(people)").vfont(16).foregroundStyle(VUI.textSecondary)
                AmountText(share, size: 54)
                Text("each pays").vfont(14).foregroundStyle(VUI.textTertiary)
                ZStack {
                    RoundedRectangle(cornerRadius: 30, style: .continuous).fill(.white).frame(width: 280, height: 280)
                    if let qr { qr.interpolation(.none).resizable().scaledToFit().frame(width: 238, height: 238).accessibilityLabel("Request QR for \(share.display)") }
                    else { Image(systemName: "qrcode").font(.system(size: 150)).foregroundStyle(.black) }
                }
                Text("Everyone scans this in Vayora Pay. It pays \(myName).").vfont(14).foregroundStyle(VUI.textSecondary).multilineTextAlignment(.center).padding(.horizontal, 20)
                Spacer(minLength: 0)
                Button(action: onDone) { Text("Done").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                    .background(.white.opacity(0.14), in: RoundedRectangle(cornerRadius: 22, style: .continuous)) }.buttonStyle(PressScale())
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}

public struct ReminderRowModel: Identifiable, Equatable, Sendable {
    public let id: String, name: String, amount: Money, schedule: String, next: String, enabled: Bool
    public init(id: String, name: String, amount: Money, schedule: String, next: String, enabled: Bool) { self.id = id; self.name = name; self.amount = amount; self.schedule = schedule; self.next = next; self.enabled = enabled }
}

public struct RemindersScreen: View {
    let rows: [ReminderRowModel], scrolls: Bool, onToggle: (ReminderRowModel) -> Void, onAdd: () -> Void, onDelete: (ReminderRowModel) -> Void
    public init(rows: [ReminderRowModel], scrolls: Bool = true, onToggle: @escaping (ReminderRowModel) -> Void = { _ in }, onAdd: @escaping () -> Void = {}, onDelete: @escaping (ReminderRowModel) -> Void = { _ in }) {
        self.rows = rows; self.scrolls = scrolls; self.onToggle = onToggle; self.onAdd = onAdd; self.onDelete = onDelete
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.8)
            if scrolls { ScrollView { content } } else { content.frame(maxHeight: .infinity, alignment: .top) }
        }
        .environment(\.colorScheme, .dark)
    }
    var content: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Bills").vfont(28, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Spacer()
                Button(action: onAdd) { Label("Add", systemImage: "plus").vfont(15, .semibold).foregroundStyle(.white).padding(.horizontal, 14).padding(.vertical, 9).background(Brand.pay.gradient, in: Capsule()) }.buttonStyle(PressScale())
            }
            if rows.isEmpty { GlassCard { Text("No reminders. Add rent, electricity or a recharge and get a nudge each month.").vfont(15).foregroundStyle(VUI.textSecondary) } }
            ForEach(rows) { r in
                HStack(spacing: 14) {
                    Image(systemName: "bell.fill").font(.system(size: 17, weight: .semibold)).foregroundStyle(r.enabled ? VUI.amber : VUI.textTertiary)
                        .frame(width: 44, height: 44).background((r.enabled ? VUI.amber : .white).opacity(0.14), in: Circle())
                    VStack(alignment: .leading, spacing: 3) {
                        Text(r.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                        Text("\(r.amount.display) · \(r.schedule)").vfont(12.5).foregroundStyle(VUI.textSecondary)
                        if r.enabled { Text("Next: \(r.next)").vfont(12).foregroundStyle(VUI.textTertiary) }
                    }
                    Spacer()
                    Button { Haptics.tick(); onToggle(r) } label: {
                        Image(systemName: r.enabled ? "checkmark.circle.fill" : "circle").font(.system(size: 24)).foregroundStyle(r.enabled ? VUI.mint : VUI.textTertiary).frame(width: 44, height: 44)
                    }.buttonStyle(.plain).accessibilityLabel(r.enabled ? "Turn reminder off" : "Turn reminder on")
                }
                .padding(.leading, 14).padding(.vertical, 8).padding(.trailing, 4)
                .background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.hairline))
                .contextMenu { Button(role: .destructive) { onDelete(r) } label: { Label("Delete", systemImage: "trash") } }
            }
            Text("Reminders are local notifications on this phone. They do not pay anything.").vfont(12).foregroundStyle(VUI.textTertiary).padding(.top, 6)
        }
        .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
    }
}
