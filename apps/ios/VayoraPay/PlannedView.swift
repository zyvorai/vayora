import SwiftUI
import VayoraKit
import VayoraUI

/// Planned payments (you confirm each one), repeating payments found in your history, and split bills still owed to you.
struct PlannedView: View {
    @EnvironmentObject var store: Store
    @State private var adding = false
    @State private var paying: ScheduledPayment?
    @State private var historyBefore = 0
    @State private var justPaid: ScheduledPayment?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                section("Planned payments") {
                    let up = ScheduleBook.upcoming(store.scheduled)
                    if up.isEmpty { Text("Nothing planned. Plan a payment for a date; you still confirm it with the slide and PIN.").vfont(14).foregroundStyle(VUI.textSecondary) }
                    ForEach(up) { p in
                        let due = ScheduleBook.due(store.scheduled, now: Date()).contains(p)
                        HStack(spacing: 12) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(p.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                                Text("\(Money(minor: p.amountMinor).display) · \(p.date.formatted(date: .abbreviated, time: .shortened))").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                            Spacer()
                            if due { Button("Pay now") { historyBefore = store.history.count; justPaid = p; paying = p }.vfont(14, .semibold).foregroundStyle(.white).padding(.horizontal, 14).padding(.vertical, 8).background(Brand.pay.gradient, in: Capsule()) }
                            Button { store.scheduled.removeAll { $0.id == p.id } } label: { Image(systemName: "trash").foregroundStyle(VUI.textTertiary) }.accessibilityLabel("Delete planned payment to \(p.name)")
                        }
                    }
                    Button { adding = true } label: { Label("Plan a payment", systemImage: "plus.circle.fill") }.vfont(15, .semibold).foregroundStyle(VUI.violet)
                }
                section("Repeating payments") {
                    let found = RecurringDetector.find(store.historyRows)
                    if found.isEmpty { Text("Payments that repeat every week or month show up here once you have three in a row.").vfont(14).foregroundStyle(VUI.textSecondary) }
                    ForEach(found, id: \.vpa) { f in
                        HStack(spacing: 12) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(f.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                                Text("\(f.cadence.title) · about \(Money(minor: f.typicalMinor).display) · next around \(f.next.formatted(date: .abbreviated, time: .omitted))").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                            Spacer()
                            Button("Plan next") {
                                store.scheduled.append(ScheduledPayment(vpa: f.vpa, name: f.name, amountMinor: f.typicalMinor, date: Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: f.next) ?? f.next))
                                Haptics.success()
                            }.vfont(13, .semibold).foregroundStyle(VUI.violet)
                        }
                    }
                    Text("Found from your own history. It only suggests; nothing is set up for you.").vfont(12).foregroundStyle(VUI.textTertiary)
                }
                section("Splits") {
                    if store.splits.isEmpty { Text("Split a bill from Home and track who has paid you back.").vfont(14).foregroundStyle(VUI.textSecondary) }
                    ForEach($store.splits) { $s in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack { Text("\(Money(minor: s.totalMinor).display) · \(s.date.formatted(date: .abbreviated, time: .omitted))").vfont(15, .semibold).foregroundStyle(VUI.textPrimary); Spacer()
                                StatusPill(s.isSettled ? "Settled" : "\(Money(minor: s.owedMinor).display) owed", s.isSettled ? .success : .warn) }
                            ForEach($s.members) { $m in
                                Toggle(isOn: $m.paid) { Text("\(m.name) · \(Money(minor: s.shareMinor).display)").vfont(14).foregroundStyle(VUI.textSecondary) }.tint(VUI.mint)
                            }
                        }
                    }
                    if !store.splits.isEmpty { Text("Tick a person when they have paid you. This phone is not told when another phone pays.").vfont(12).foregroundStyle(VUI.textTertiary) }
                }
            }
            .padding(20)
        }
        .background(VUI.ink)
        .sheet(isPresented: $adding) { PlanPaymentSheet().environmentObject(store) }
        .sheet(item: $paying, onDismiss: markPaid) { p in
            PaySheet(payload: UPIPayload(payee: p.vpa, name: p.name, amount: Money(minor: p.amountMinor), note: p.note), transfer: true).environmentObject(store)
        }
    }

    /// Done only when a new successful payment to that person was recorded while the sheet was open.
    func markPaid() {
        defer { justPaid = nil }
        guard let p = justPaid, store.history.count > historyBefore, let t = store.history.first, t.status == .success, t.payee == p.vpa,
              let i = store.scheduled.firstIndex(where: { $0.id == p.id }) else { return }
        store.scheduled[i].done = true
    }

    func section<C: View>(_ title: String, @ViewBuilder _ content: () -> C) -> some View {
        GlassCard { VStack(alignment: .leading, spacing: 12) { Text(title).vfont(17, .semibold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader); content() } }
    }
}

struct PlanPaymentSheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var payee: Payee?
    @State private var amount = ""
    @State private var date = Calendar.current.date(byAdding: .day, value: 1, to: Date()) ?? Date()

    var money: Money? { Money(decimal: amount) }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.8)
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text("Plan a payment").vfont(24, .bold).foregroundStyle(VUI.textPrimary); Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        if store.payees.all.isEmpty { Text("Save a payee first (Payees tab).").vfont(14).foregroundStyle(VUI.textSecondary) }
                        ForEach(store.payees.sorted) { p in
                            Button { payee = p } label: { HStack { Text(p.name).foregroundStyle(VUI.textPrimary); Spacer(); if payee == p { Image(systemName: "checkmark.circle.fill").foregroundStyle(VUI.mint) } }.vfont(16) }
                        }
                        TextField("Amount in ₹", text: $amount).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                        DatePicker("When", selection: $date, in: Date()..., displayedComponents: [.date, .hourAndMinute]).vfont(16).foregroundStyle(VUI.textPrimary)
                        Text("You get a notification then. It reminds you; it never pays anything.").vfont(12).foregroundStyle(VUI.textTertiary)
                    }
                }
                Button {
                    guard let p = payee, let m = money else { return }
                    store.scheduled.append(ScheduledPayment(vpa: p.vpa, name: p.name, amountMinor: m.minor, date: date)); Haptics.success(); dismiss()
                } label: {
                    Text("Save").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17).background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                }.buttonStyle(PressScale()).opacity(payee != nil && money != nil ? 1 : 0.4).disabled(payee == nil || money == nil)
                Spacer()
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}
