import SwiftUI
import VayoraKit
import VayoraUI

/// Bills holds two things the user sets up once and then forgets: reminders (they never pay) and AutoPay (they do).
struct BillsView: View {
    enum Mode: String, CaseIterable { case reminders = "Reminders", autopay = "AutoPay" }
    @State private var mode = Mode.reminders
    var body: some View {
        VStack(spacing: 0) {
            Picker("Bills", selection: $mode) { ForEach(Mode.allCases, id: \.self) { Text($0.rawValue).tag($0) } }
                .pickerStyle(.segmented).padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 4)
            switch mode {
            case .reminders: RemindersView()
            case .autopay: AutoPayView()
            }
        }
        .background(VUI.ink)
    }
}

struct AutoPayView: View {
    @EnvironmentObject var store: Store
    @State private var scanning = false
    @State private var mandate: (MandatePayload, String)?
    @State private var problem: String?
    @State private var stopping: PayerMandateCardModel?

    func cards() -> [PayerMandateCardModel] {
        store.autoPays.map { m in
            let taken = m.debits.reduce(Int64(0)) { $0 + $1.amountMinor }
            let freq = MandateFrequency(rawValue: m.frequency)?.title ?? m.frequency
            return PayerMandateCardModel(
                id: m.id, merchant: m.merchant, payee: m.payee, cap: Money(minor: m.capMinor), frequency: freq, until: m.until,
                status: MandateStatusKind(status: m.status), taken: Money(minor: taken),
                recent: m.debits.reversed().map { DebitRowModel(id: $0.attemptID, amount: Money(minor: $0.amountMinor), when: $0.at.prefix(10).description, state: "captured") })
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Button { scanning = true } label: {
                    Label("Scan an AutoPay QR", systemImage: "qrcode.viewfinder").vfont(16, .semibold).foregroundStyle(.white)
                        .frame(maxWidth: .infinity).padding(.vertical, 15).background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                }.buttonStyle(PressScale())
                PayerAutoPayList(cards: cards(), onRevoke: { stopping = $0 })
                Text("Test mode: the simulator plays your bank. Real UPI AutoPay needs a bank and NPCI.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
            .padding(20)
        }
        .background(VUI.ink)
        .refreshable { await store.refreshAutoPays() }
        .task { await store.refreshAutoPays() }
        .fullScreenCover(isPresented: $scanning) {
            ScannerView(brand: .pay, hint: "Scan the AutoPay QR") { text in
                scanning = false
                if let host = store.applySetup(text) { problem = "Test server set to \(host)."; return }
                do { mandate = (try MandatePayload.parse(text), text) } catch { problem = "That is not a valid AutoPay QR." }
            } onCancel: { scanning = false }
        }
        .sheet(isPresented: Binding(get: { mandate != nil }, set: { if !$0 { mandate = nil; Task { await store.refreshAutoPays() } } })) {
            if let m = mandate { MandateApproveSheet(mandate: m.0, uri: m.1).environmentObject(store) }
        }
        .alert(problem?.hasPrefix("Test server set") == true ? "Done" : "Can't read that code", isPresented: Binding(get: { problem != nil }, set: { _ in problem = nil })) { Button("OK", role: .cancel) {} } message: { Text(problem ?? "") }
        .confirmationDialog("Stop this AutoPay?", isPresented: Binding(get: { stopping != nil }, set: { if !$0 { stopping = nil } }), titleVisibility: .visible) {
            Button("Stop AutoPay", role: .destructive) {
                if let s = stopping { Task { if !(await store.stopAutoPay(s.id)) { problem = "Couldn't stop it. Check the test server." } } }
            }
        } message: { Text("They will not be able to take any more money.") }
    }
}
