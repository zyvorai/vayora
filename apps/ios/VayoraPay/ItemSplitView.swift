import SwiftUI
import VayoraKit
import VayoraUI

/// Split a bill by item: name the people, add items, tick who shared each, then show each person their own request QR.
struct ItemSplitView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var people = ["Friend 1", "Friend 2"]
    @State private var items: [Row] = [Row()]
    @State private var showing: Int?
    @State private var saved = false

    struct Row: Identifiable { var id = UUID(); var name = ""; var amount = ""; var who = Set<Int>() }

    var parsed: [ItemSplit.Item] { items.compactMap { r in Money(decimal: r.amount).map { ItemSplit.Item(name: r.name, amountMinor: $0.minor, people: r.who) } } }
    /// Index 0 is you; the rest are your friends in order.
    var everyone: [String] { [store.myName.isEmpty ? "You" : store.myName] + people }
    var shares: [Int64] { ItemSplit.shares(parsed, peopleCount: everyone.count) }
    var total: Int64 { parsed.reduce(0) { $0 + $1.amountMinor } }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Split by item").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    GlassCard {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("People").vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                            ForEach(people.indices, id: \.self) { i in
                                HStack { TextField("Name", text: $people[i]).foregroundStyle(VUI.textPrimary).padding(10).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                                    if people.count > 1 { Button { removePerson(i) } label: { Image(systemName: "minus.circle").foregroundStyle(VUI.textTertiary) }.accessibilityLabel("Remove \(people[i])") } }
                            }
                            if people.count < 7 { Button { people.append("Friend \(people.count + 1)") } label: { Label("Add a person", systemImage: "plus.circle") }.vfont(14, .semibold).foregroundStyle(VUI.violet) }
                            Text("You are person one. Items left unticked are shared by everyone.").vfont(12).foregroundStyle(VUI.textTertiary)
                        }
                    }
                    ForEach($items) { $r in
                        GlassCard {
                            VStack(alignment: .leading, spacing: 8) {
                                HStack { TextField("Item", text: $r.name).foregroundStyle(VUI.textPrimary)
                                    TextField("₹", text: $r.amount).keyboardType(.decimalPad).multilineTextAlignment(.trailing).frame(width: 90).foregroundStyle(VUI.textPrimary) }
                                ScrollView(.horizontal, showsIndicators: false) { HStack(spacing: 6) {
                                    ForEach(everyone.indices, id: \.self) { i in
                                        Button { if r.who.contains(i) { r.who.remove(i) } else { r.who.insert(i) } } label: {
                                            Text(everyone[i]).vfont(12, .semibold).foregroundStyle(r.who.contains(i) ? .white : VUI.textSecondary).padding(.horizontal, 10).padding(.vertical, 6)
                                                .background(r.who.contains(i) ? AnyShapeStyle(VUI.violet) : AnyShapeStyle(.white.opacity(0.08)), in: Capsule())
                                        }.accessibilityAddTraits(r.who.contains(i) ? .isSelected : [])
                                    } } }
                            }
                        }
                    }
                    Button { items.append(Row()) } label: { Label("Add an item", systemImage: "plus.circle.fill") }.vfont(15, .semibold).foregroundStyle(VUI.violet)
                    GlassCard {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack { Text("Total").foregroundStyle(VUI.textSecondary); Spacer(); Text(Money(minor: total).display).foregroundStyle(VUI.textPrimary).fontWeight(.semibold) }.vfont(16)
                            ForEach(everyone.indices, id: \.self) { i in
                                HStack {
                                    Text(everyone[i]).foregroundStyle(VUI.textPrimary); Spacer()
                                    Text(Money(minor: shares[i]).display).foregroundStyle(VUI.textSecondary)
                                    if i > 0, shares[i] > 0 { Button("QR") { showing = i }.vfont(13, .semibold).foregroundStyle(VUI.violet) }
                                }.vfont(15)
                            }
                            Text("The shares add up to exactly the total. Left-over paise go to the first people in the list.").vfont(12).foregroundStyle(VUI.textTertiary)
                        }
                    }
                    Button(action: save) { Text(saved ? "Saved to Splits" : "Save to Splits").vfont(17, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 15).background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 20, style: .continuous)) }
                        .buttonStyle(PressScale()).opacity(total > 0 && !saved ? 1 : 0.4).disabled(total == 0 || saved)
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .sheet(item: Binding(get: { showing.map { Pick(i: $0) } }, set: { if $0 == nil { showing = nil } })) { p in
            VStack(spacing: 16) {
                Text("\(everyone[p.i]) pays \(Money(minor: shares[p.i]).display)").vfont(20, .bold).foregroundStyle(VUI.textPrimary)
                QR.image(RequestPayload.make(payee: store.myVPA, name: store.myName, amount: Money(minor: shares[p.i]), note: "Split"))?.interpolation(.none).resizable().scaledToFit().frame(maxWidth: 280).padding(16).background(.white, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                Text("Their Vayora Pay scans this. Test money.").vfont(13).foregroundStyle(VUI.textSecondary)
                Button("Done") { showing = nil }.vfont(16, .semibold).foregroundStyle(VUI.violet)
            }.padding(24).frame(maxWidth: .infinity, maxHeight: .infinity).background(VUI.ink).preferredColorScheme(.dark)
        }
    }

    struct Pick: Identifiable { let i: Int; var id: Int { i } }

    func removePerson(_ i: Int) {
        people.remove(at: i)
        // shift the ticks so they keep pointing at the same people
        for k in items.indices { items[k].who = Set(items[k].who.compactMap { $0 == i + 1 ? nil : ($0 > i + 1 ? $0 - 1 : $0) }) }
    }

    func save() {
        let owing = people.indices.map { (name: people[$0].trimmingCharacters(in: .whitespaces).isEmpty ? "Friend \($0 + 1)" : people[$0], shareMinor: shares[$0 + 1]) }.filter { $0.shareMinor > 0 }
        store.splits.insert(SplitRecord(date: Date(), note: "By item", totalMinor: total, owing: owing), at: 0)
        saved = true; Haptics.success()
    }
}
