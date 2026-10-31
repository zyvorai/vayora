import SwiftUI
import VayoraKit
import VayoraUI

/// AutoPay: the list, a new AutoPay (details, cap, QR), and a mandate's detail with debits.
struct AutoPayView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var mandates: [MandateInfo] = []
    @State private var names: [String: String] = [:]
    @State private var error: String?
    @State private var creating = false
    @State private var open: MandateInfo?

    var rows: [MandateRowModel] {
        mandates.map { m in
            MandateRowModel(id: m.id, name: names[m.customerID] ?? "Customer", line: "Up to \(m.cap.display) · \(m.frequencyTitle)", status: MandateStatusKind(status: m.status, revokedBy: m.revokedBy))
        }
    }

    var body: some View {
        NavigationStack {
            AutoPayListScreen(rows: rows, error: store.client == nil ? "Add your API key in Settings first." : error,
                              onOpen: { r in open = mandates.first { $0.id == r.id } }, onNew: { creating = true })
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(item: $open) { m in MandateDetailView(mandate: m, name: names[m.customerID] ?? "Customer") { await load() }.environmentObject(store) }
        }
        .task { await load() }
        .fullScreenCover(isPresented: $creating, onDismiss: { Task { await load() } }) { NewAutoPayFlow().environmentObject(store) }
    }

    func load() async {
        guard let c = store.client else { return }
        do {
            async let ms = c.mandates(), cs = c.customers()
            let (m, customers) = try await (ms, cs)
            names = Dictionary(customers.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
            mandates = m; error = nil
        } catch { self.error = (error as? APIError)?.message ?? "Couldn't reach the server." }
    }
}

func isoDay(_ d: Date) -> String {
    let c = Calendar.current.dateComponents([.year, .month, .day], from: d)
    return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
}

struct NewAutoPayFlow: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    enum Step: Equatable { case details, cap, creating, qr(MandateInfo), failed(String) }
    @State private var step = Step.details
    @State private var name = ""
    @State private var frequency = MandateFrequency.monthly
    @State private var months = 12
    @State private var entry = AmountEntry()
    @State private var status = MandateStatusKind.waiting
    @State private var approved = 0.0

    var endDate: Date { Calendar.current.date(byAdding: .month, value: months, to: Date()) ?? Date() }
    var trimmed: String { name.trimmingCharacters(in: .whitespaces) }

    var body: some View {
        ZStack(alignment: .topLeading) {
            switch step {
            case .details:
                NewAutoPayDetailsScreen(name: $name, frequency: $frequency, months: $months, canContinue: !trimmed.isEmpty && store.client != nil) { withAnimation(VUI.spring) { step = .cap } }
            case .cap:
                CollectKeypadScreen(text: entry.text, note: "", chips: [], canContinue: entry.money != nil, title: "Most per debit", actionLabel: "Create AutoPay", actionIcon: "arrow.right",
                                    onDigit: { entry.digit($0) }, onPoint: { entry.point() }, onDelete: { entry.delete() }, onShowQR: create)
            case .creating:
                ZStack { AmbientBackground(brand: .merchant); ProgressView().controlSize(.large).tint(.white) }
            case .qr(let m):
                TimelineView(.periodic(from: .now, by: 3)) { _ in
                    MandateQRScreen(customer: trimmed, cap: m.cap, frequency: m.frequencyTitle, until: m.endDate, qr: m.authorization?.uri.map { QR.image($0) } ?? nil,
                                    status: status, approved: approved, onDone: { dismiss() })
                }
                .task(id: m.id) { await watch(m) }
            case .failed(let why):
                ResultScreen(kind: .failed(why), amount: Money(minor: 0), to: "", progress: 1, brand: .merchant, doneLabel: "Back", onDone: { step = .details })
            }
            if case .qr = step {} else if step != .creating {
                Button { dismiss() } label: {
                    Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle())
                }
                .padding(.leading, 18).padding(.top, 14).accessibilityLabel("Close")
            }
        }
        .background(VUI.ink)
        .preferredColorScheme(.dark)
    }

    func create() {
        guard let c = store.client, let cap = entry.money else { return }
        step = .creating
        Task {
            do {
                let customer = try await c.customer(named: trimmed)
                let m = try await c.createMandate(customerID: customer.id, cap: cap, frequency: frequency, endDate: isoDay(endDate))
                step = .qr(m)
            } catch { step = .failed((error as? APIError)?.message ?? "Couldn't reach the server.") }
        }
    }

    /// Ask the server whether the payer has approved yet, until they have (or this screen goes away).
    func watch(_ m: MandateInfo) async {
        guard let c = store.client else { return }
        while !Task.isCancelled {
            if let r = try? await c.refreshMandate(id: m.id) {
                status = MandateStatusKind(status: r.status, revokedBy: r.revokedBy)
                if r.status == "active" { withAnimation(.easeOut(duration: 1.0)) { approved = 1 }; Haptics.success(); return }
                if !r.isLive { return }
            }
            try? await Task.sleep(nanoseconds: 2_000_000_000)
        }
    }
}

struct MandateDetailView: View {
    @EnvironmentObject var store: MerchantStore
    @State var mandate: MandateInfo
    let name: String
    let reload: () async -> Void
    @State private var debits: [MandateDebitInfo] = []
    @State private var note: String?
    @State private var scheduling = false
    @State private var confirmRevoke = false

    var line: String { "Up to \(mandate.cap.display) · \(mandate.frequencyTitle) · until \(mandate.endDate)" }

    var body: some View {
        MandateDetailScreen(
            name: name, cap: mandate.cap, line: line, status: MandateStatusKind(status: mandate.status, revokedBy: mandate.revokedBy), note: note,
            debits: debits.map { DebitRowModel(id: $0.id, amount: $0.money, when: $0.dueDate, state: $0.state) },
            onSchedule: { scheduling = true }, onRunNow: runNow,
            onPauseResume: { act { c in mandate.status == "paused" ? try await c.resumeMandate(id: mandate.id) : try await c.pauseMandate(id: mandate.id) } },
            onRevoke: { confirmRevoke = true })
        .toolbar(.hidden, for: .navigationBar)
        .task { await refresh() }
        .refreshable { await refresh() }
        .sheet(isPresented: $scheduling, onDismiss: { Task { await refresh() } }) { ScheduleDebitSheet(mandate: mandate).environmentObject(store) }
        .confirmationDialog("Stop this AutoPay?", isPresented: $confirmRevoke, titleVisibility: .visible) {
            Button("Revoke", role: .destructive) { act { c in try await c.revokeMandate(id: mandate.id) } }
        } message: { Text("No more debits can be taken. This can't be undone.") }
    }

    func refresh() async {
        guard let c = store.client else { return }
        if let r = try? await c.refreshMandate(id: mandate.id) { mandate = r }
        if let d = try? await c.mandate(id: mandate.id) { mandate = d.mandate; debits = d.debits }
        await reload()
    }

    func act(_ f: @escaping (VayoraClient) async throws -> MandateInfo) {
        guard let c = store.client else { return }
        Task {
            do { mandate = try await f(c); note = nil; Haptics.success() } catch { note = (error as? APIError)?.message ?? "Couldn't reach the server."; Haptics.error() }
            await refresh()
        }
    }

    /// Sandbox: the simulator only moves when the API runs its pass, so do it on demand.
    func runNow() {
        guard let c = store.client else { return }
        Task {
            do {
                let r = try await c.runMandates()
                note = "Pass ran: \(r.notified) announced, \(r.initiated) sent."
            } catch { note = (error as? APIError)?.message ?? "Couldn't reach the server." }
            await refresh()
        }
    }
}

struct ScheduleDebitSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    let mandate: MandateInfo
    @State private var amount = ""
    @State private var due = Date()
    @State private var problem: String?
    @State private var working = false

    var money: Money? { Money(decimal: amount) }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.8)
            VStack(alignment: .leading, spacing: 16) {
                Text("Schedule a debit").vfont(24, .bold).foregroundStyle(VUI.textPrimary)
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        TextField("Amount in ₹ (up to \(mandate.cap.display))", text: $amount).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary)
                            .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                        DatePicker("Due", selection: $due, in: Date()..., displayedComponents: .date).vfont(16).foregroundStyle(VUI.textPrimary)
                        Text("The payer is told first, then it is taken on the due date. In this sandbox, use Run now on the AutoPay.").vfont(12).foregroundStyle(VUI.textTertiary)
                        if let problem { Text(problem).vfont(13).foregroundStyle(VUI.coral) }
                    }
                }
                Button(action: schedule) {
                    Text(working ? "Scheduling…" : "Schedule").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                        .background(Brand.merchant.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                }.buttonStyle(PressScale()).opacity(money != nil && !working ? 1 : 0.4).disabled(money == nil || working)
                Button("Cancel") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity)
                Spacer()
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }

    func schedule() {
        guard let c = store.client, let m = money else { return }
        working = true
        Task {
            do { _ = try await c.scheduleDebit(mandateID: mandate.id, amount: m, due: isoDay(due)); Haptics.success(); dismiss() }
            catch { problem = (error as? APIError)?.message ?? "Couldn't reach the server."; Haptics.error() }
            working = false
        }
    }
}
