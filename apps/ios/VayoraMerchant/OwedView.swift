import SwiftUI
import VayoraKit
import VayoraUI

/// What customers owe: unpaid invoices by how late they are, and reminders waiting for a person's yes.
struct OwedView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var ageing: Ageing?
    @State private var drafts: [ReminderDraft] = []
    @State private var approvals: [PendingApproval] = []
    @State private var message: String?
    @State private var notice: String?
    @State private var promising: OwedInvoice?

    var body: some View {
        NavigationStack {
            ZStack {
                AmbientBackground(brand: .merchant, phase: 0.4)
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        Text("Owed to you").vfont(28, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        if let a = ageing {
                            GlassCard {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text("Unpaid in total").vfont(13).foregroundStyle(VUI.textSecondary)
                                    Text(Money(minor: a.inrTotalMinor).display).vfont(32, .bold).foregroundStyle(VUI.mint)
                                    Text("As of \(a.asOf). Rupee invoices only.").vfont(12).foregroundStyle(VUI.textTertiary)
                                }
                            }
                        }
                        if let message { Text(message).vfont(13).foregroundStyle(VUI.coral).accessibilityLabel("Error: \(message)") }
                        if let notice { Text(notice).vfont(13).foregroundStyle(VUI.mint) }
                        if !approvals.isEmpty { approvalsCard }
                        if !drafts.isEmpty { draftsCard }
                        if let a = ageing {
                            if a.inrBuckets.isEmpty { GlassCard { Text("Nothing is owed right now.").vfont(14).foregroundStyle(VUI.textSecondary) } }
                            ForEach(a.inrBuckets) { b in bucketCard(b) }
                        }
                        if store.demoMode { Text("Test money. The built-in demo has sample invoices and cannot email.").vfont(12).foregroundStyle(VUI.textTertiary) }
                    }.padding(20)
                }
                .refreshable { await load() }
            }
            .environment(\.colorScheme, .dark)
        }
        .task { await load() }
        .sheet(item: $promising) { inv in PromiseSheet(invoice: inv) { notice = "Promise saved for \(inv.number)."; Task { await load() } }.environmentObject(store) }
    }

    /// Refunds and payouts someone else asked for. The server refuses the person who asked, and anyone without the approve permission.
    var approvalsCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Waiting for your approval", systemImage: "checkmark.shield").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                ForEach(approvals) { a in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text("\(a.title) · \(Money(minor: a.amountMinor).display)").vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            if !a.requestedBy.isEmpty { Text("by \(a.requestedBy)").vfont(12).foregroundStyle(VUI.textTertiary) }
                        }
                        if !a.detail.isEmpty { Text(a.detail).vfont(13).foregroundStyle(VUI.textSecondary) }
                        HStack(spacing: 16) {
                            Button("Approve") { decide(a, approve: true) }.foregroundStyle(VUI.mint)
                            Button("Reject", role: .destructive) { decide(a, approve: false) }
                        }.vfont(14, .semibold)
                        .accessibilityHint(a.kind == .payout ? "Approving lets the payout go ahead" : "Approving sends the refund to the customer")
                    }
                    if a.id != approvals.last?.id { Divider().overlay(.white.opacity(0.1)) }
                }
            }
        }
    }

    var draftsCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Reminders to approve", systemImage: "text.badge.checkmark").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                ForEach(drafts) { d in
                    VStack(alignment: .leading, spacing: 6) {
                        Text([d.invoiceNumber, d.channel].compactMap { $0 }.joined(separator: " · ")).vfont(12, .semibold).foregroundStyle(VUI.textTertiary)
                        Text(d.message).vfont(14).foregroundStyle(VUI.textPrimary)
                        HStack(spacing: 16) {
                            Button("Approve") { decide(d, approve: true) }.foregroundStyle(VUI.mint)
                            Button("Reject", role: .destructive) { decide(d, approve: false) }
                        }.vfont(14, .semibold)
                        .accessibilityHint("Approving sends the reminder as written")
                    }
                    if d.id != drafts.last?.id { Divider().overlay(.white.opacity(0.1)) }
                }
            }
        }
    }

    func bucketCard(_ b: AgeingBucket) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(b.title).vfont(16, .semibold).foregroundStyle(b.rank <= 2 ? VUI.coral : VUI.textPrimary)
                    Spacer()
                    Text(Money(minor: b.outstandingMinor).display).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                }
                ForEach(b.invoices) { i in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(i.number).vfont(14, .semibold).foregroundStyle(VUI.textPrimary)
                            Text(i.dueDate.map { "Due \($0)" } ?? "No due date").vfont(12).foregroundStyle(VUI.textSecondary)
                        }
                        Spacer()
                        Text(Money(minor: i.outstandingMinor).display).vfont(14).foregroundStyle(VUI.textPrimary)
                        Menu {
                            Button("Email invoice") { email(i) }
                            Button("Customer promised to pay…") { promising = i }
                        } label: { Image(systemName: "ellipsis.circle").font(.system(size: 20)).foregroundStyle(VUI.mint) }
                            .accessibilityLabel("Actions for \(i.number)")
                    }
                }
            }
        }
    }

    func load() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; return }
        do {
            async let a = c.ageing(); async let d = c.reminderDrafts()
            (ageing, drafts) = try await (a, d); message = nil
            // a key without approval rights may not read refunds or payouts: that must not hide the rest of the screen
            approvals = (try? await c.pendingApprovals()) ?? []
        } catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
    }
    func decide(_ a: PendingApproval, approve: Bool) {
        guard let c = store.client else { return }
        Task {
            do { try await c.decide(a, approve: approve); notice = "\(a.title) \(approve ? "approved" : "rejected")."; message = nil; Haptics.success(); await load() }
            catch { message = (error as? APIError)?.message ?? "Couldn't save that."; notice = nil; Haptics.error() }
        }
    }
    func decide(_ d: ReminderDraft, approve: Bool) {
        guard let c = store.client else { return }
        Task {
            do {
                if approve { try await c.approveReminderDraft(id: d.id) } else { try await c.rejectReminderDraft(id: d.id) }
                notice = approve ? "Reminder approved." : "Reminder rejected."; Haptics.success(); await load()
            } catch { message = (error as? APIError)?.message ?? "Couldn't save that."; notice = nil; Haptics.error() }
        }
    }
    func email(_ i: OwedInvoice) {
        guard let c = store.client else { return }
        Task {
            do { try await c.emailInvoice(id: i.id); notice = "Invoice \(i.number) emailed."; message = nil; Haptics.success() }
            catch { message = (error as? APIError)?.message ?? "Couldn't email the invoice."; notice = nil; Haptics.error() }
        }
    }
}

/// Records that a customer said they will pay a given amount on a given day.
struct PromiseSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    let invoice: OwedInvoice
    var onSaved: () -> Void
    @State private var date = Calendar.current.date(byAdding: .day, value: 3, to: Date()) ?? Date()
    @State private var note = ""
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.8)
            VStack(alignment: .leading, spacing: 14) {
                Text("Promise to pay").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Text("\(invoice.number) · \(Money(minor: invoice.outstandingMinor).display) still owed").vfont(14).foregroundStyle(VUI.textSecondary)
                DatePicker("Pays on", selection: $date, in: Date()..., displayedComponents: .date).foregroundStyle(VUI.textPrimary).tint(VUI.mint)
                TextField("Note (optional)", text: $note).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                Button(action: save) { Text("Save promise").vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 14).background(VUI.mint, in: Capsule()) }.buttonStyle(PressScale())
                Button("Cancel") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity)
                Spacer()
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.medium])
    }

    func save() {
        guard let c = store.client else { return }
        let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd"; f.locale = Locale(identifier: "en_US_POSIX")
        Task {
            do {
                try await c.promiseToPay(invoiceID: invoice.id, amount: Money(minor: invoice.outstandingMinor), date: f.string(from: date), note: note.trimmingCharacters(in: .whitespaces))
                Haptics.success(); onSaved(); dismiss()
            } catch { message = (error as? APIError)?.message ?? "Couldn't save the promise."; Haptics.error() }
        }
    }
}
