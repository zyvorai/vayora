import SwiftUI
import VayoraKit
import VayoraUI

/// Payees: tap one to send them a test transfer. Used as a tab and as the Send picker.
struct PayeesView: View {
    @EnvironmentObject var store: Store
    @State private var sending: Payee?
    @State private var adding = false

    var body: some View {
        PayeesScreen(payees: store.payees.sorted, onOpen: { sending = $0 }, onStar: { store.payees.toggleFavourite($0.vpa) }, onAdd: { adding = true })
            .sheet(item: $sending) { p in
                PaySheet(payload: UPIPayload(payee: p.vpa, name: p.name, amount: nil, note: ""), transfer: true).environmentObject(store)
            }
            .sheet(isPresented: $adding) { AddPayeeSheet().environmentObject(store) }
    }
}

struct AddPayeeSheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var name = ""
    @State private var input = ""
    @State private var problem: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.62)
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text("New payee").vfont(24, .bold).foregroundStyle(VUI.textPrimary); Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        field("Name", text: $name, keyboard: .default)
                        field("UPI ID or mobile number", text: $input, keyboard: .emailAddress)
                        if let problem { Text(problem).vfont(13).foregroundStyle(VUI.coral) }
                        Text("A mobile number is kept as <number>@\(PayeeInput.phoneDomain). Test mode has no bank to look numbers up in.").vfont(12).foregroundStyle(VUI.textTertiary)
                    }
                }
                Button(action: save) {
                    Text("Save payee").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                        .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                }.buttonStyle(PressScale())
                Spacer()
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }

    func field(_ title: String, text: Binding<String>, keyboard: UIKeyboardType) -> some View {
        TextField(title, text: text).textInputAutocapitalization(keyboard == .default ? .words : .never).autocorrectionDisabled().keyboardType(keyboard)
            .foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    func save() {
        if store.payees.save(name: name, input: input) { Haptics.success(); dismiss() }
        else { problem = "That is not a UPI ID (like name@bank) or a 10-digit Indian mobile number."; Haptics.error() }
    }
}
