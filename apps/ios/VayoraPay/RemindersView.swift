import SwiftUI
import VayoraKit
import VayoraUI

struct RemindersView: View {
    @EnvironmentObject var store: Store
    @State private var adding = false

    func rows() -> [ReminderRowModel] {
        store.reminders.map { r in
            ReminderRowModel(id: r.id.uuidString, name: r.name, amount: Money(minor: r.amountMinor), schedule: "\(ordinal(r.dayOfMonth)) of each month",
                             next: r.nextDue(after: Date()).formatted(.dateTime.day().month(.abbreviated).hour().minute()), enabled: r.enabled)
        }
    }
    func ordinal(_ n: Int) -> String { let s = ["th", "st", "nd", "rd"]; let v = n % 100; return "\(n)" + (v >= 11 && v <= 13 ? "th" : s[min(n % 10, 4) < 4 ? n % 10 : 0]) }

    var body: some View {
        RemindersScreen(rows: rows(), onToggle: { row in
            if let i = store.reminders.firstIndex(where: { $0.id.uuidString == row.id }) { store.reminders[i].enabled.toggle() }
        }, onAdd: { adding = true }, onDelete: { row in store.reminders.removeAll { $0.id.uuidString == row.id } })
        .sheet(isPresented: $adding) { AddReminderSheet().environmentObject(store) }
    }
}

struct AddReminderSheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var name = ""
    @State private var amount = ""
    @State private var day = 5

    var money: Money? { Money(decimal: amount) }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.8)
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text("New reminder").vfont(24, .bold).foregroundStyle(VUI.textPrimary); Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        TextField("Name (Rent, Electricity…)", text: $name).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                        TextField("Amount in ₹", text: $amount).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                        Stepper("Day of the month: \(day)", value: $day, in: 1...28).vfont(16).foregroundStyle(VUI.textPrimary)
                        Text("You get a notification at 9 am. It reminds you; it never pays anything.").vfont(12).foregroundStyle(VUI.textTertiary)
                    }
                }
                Button {
                    guard let m = money, !name.trimmingCharacters(in: .whitespaces).isEmpty else { Haptics.error(); return }
                    store.reminders.append(BillReminder(name: name.trimmingCharacters(in: .whitespaces), amountMinor: m.minor, dayOfMonth: day)); Haptics.success(); dismiss()
                } label: {
                    Text("Save reminder").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                        .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                }.buttonStyle(PressScale()).opacity(money != nil && !name.isEmpty ? 1 : 0.4)
                Spacer()
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}
