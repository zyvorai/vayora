import SwiftUI
import VayoraKit
import VayoraUI

/// What customers owe: unpaid invoices by how late they are, and reminders waiting for a person's yes.
struct OwedView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var ageing: Ageing?
    @State private var drafts: [ReminderDraft] = []
    @State private var approvals: [PendingApproval] = []
    @State private var disputes: [DisputeInfo] = []
    @State private var toPay: [ApprovedPayout] = []
    @State private var failed: [FailedRefund] = []
    @State private var paying: ApprovedPayout?
    @State private var cases: [ReconCase] = []
    @State private var caseWork: CaseWork?
    @State private var settlementShown: String?
    @State private var noting: DisputeInfo?
    @State private var deciding: DisputeInfo?
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
                        if let line = store.attention.summary { Text("Waiting: \(line)").vfont(13, .semibold).foregroundStyle(VUI.amber).accessibilityLabel("Waiting for you: \(line)") }
                        if !approvals.isEmpty { approvalsCard }
                        if !toPay.isEmpty { payCard }
                        if !cases.isEmpty { casesCard }
                        if !failed.isEmpty { failedCard }
                        if !disputes.isEmpty { disputesCard }
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
            .safeAreaInset(edge: .top, spacing: 0) { FeedbackBanner(message: $message, notice: $notice) }
            .environment(\.colorScheme, .dark)
        }
        .task { await load() }
        .sheet(item: $noting) { d in EvidenceSheet(dispute: d) { notice = "Evidence added."; Task { await load() } }.environmentObject(store) }
        .confirmationDialog("Record the issuer's decision", isPresented: Binding(get: { deciding != nil }, set: { if !$0 { deciding = nil } }), titleVisibility: .visible, presenting: deciding) { d in
            Button("We won: the hold is released") { resolve(d, won: true) }
            Button("We lost: book the loss", role: .destructive) { resolve(d, won: false) }
            Button("Cancel", role: .cancel) {}
        } message: { d in Text("\(Money(minor: d.amountMinor).display). A loss is booked on your ledger and cannot be undone here.") }
        .sheet(item: Binding(get: { settlementShown.map { SettlementRef(id: $0) } }, set: { settlementShown = $0?.id })) { SettlementSheet(settlementID: $0.id).environmentObject(store) }
        .sheet(item: $caseWork) { w in CaseSheet(work: w) { notice = w.resolving ? "Case resolved." : "Case assigned."; Task { await load() } }.environmentObject(store) }
        .sheet(item: $paying) { p in MarkPaidSheet(payout: p) { notice = "Payout marked paid."; Task { await load() } }.environmentObject(store) }
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
                            if !a.requestedBy.isEmpty { Text("by \(a.requestedBy)").vfont(12).foregroundStyle(VUI.textTertiary).lineLimit(1).truncationMode(.middle).frame(maxWidth: 140, alignment: .trailing) }
                        }
                        if !a.detail.isEmpty { Text(a.detail).vfont(13).foregroundStyle(VUI.textSecondary) }
                        HStack(spacing: 16) {
                            Button("Approve") { decide(a, approve: true) }.foregroundStyle(VUI.mint).accessibilityLabel("Approve \(a.title.lowercased()) of \(Money(minor: a.amountMinor).display)")
                            Button("Reject", role: .destructive) { decide(a, approve: false) }.accessibilityLabel("Reject \(a.title.lowercased()) of \(Money(minor: a.amountMinor).display)")
                        }.vfont(14, .semibold)
                        .accessibilityHint(a.kind == .payout ? "Approving lets the payout go ahead" : "Approving sends the refund to the customer")
                    }
                    if a.id != approvals.last?.id { Divider().overlay(.white.opacity(0.1)) }
                }
            }
        }
    }

    static func todayIST() -> String {
        let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd"; f.locale = Locale(identifier: "en_US_POSIX"); f.timeZone = TimeZone(identifier: "Asia/Kolkata")
        return f.string(from: Date())
    }

    var disputesCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Disputes to answer", systemImage: "exclamationmark.bubble").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                ForEach(disputes) { d in
                    let left = d.daysLeft(today: Self.todayIST())
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(Money(minor: d.amountMinor).display).vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            if let left { Text(left < 0 ? "Deadline passed" : left == 0 ? "Due today" : "\(left) day\(left == 1 ? "" : "s") left").vfont(12, .semibold).foregroundStyle(left <= 2 ? VUI.coral : VUI.textSecondary) }
                        }
                        Text(d.reason).vfont(13).foregroundStyle(VUI.textSecondary)
                        Text("\(d.evidenceCount) note\(d.evidenceCount == 1 ? "" : "s") added").vfont(12).foregroundStyle(VUI.textTertiary)
                        HStack(spacing: 16) {
                            Button("Add evidence") { noting = d }.foregroundStyle(VUI.mint).accessibilityLabel("Add evidence to the \(Money(minor: d.amountMinor).display) dispute")
                            Button("Record outcome") { deciding = d }.foregroundStyle(VUI.textSecondary).accessibilityLabel("Record the outcome of the \(Money(minor: d.amountMinor).display) dispute")
                        }.vfont(14, .semibold)
                    }
                    if d.id != disputes.last?.id { Divider().overlay(.white.opacity(0.1)) }
                }
                Text("Evidence is a written note and, if you have one, a link to the document on your own storage. Photos cannot be uploaded from here.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
        }
    }

    func resolve(_ d: DisputeInfo, won: Bool) {
        guard let c = store.client else { return }
        Task {
            do { try await c.resolveDispute(id: d.id, won: won); notice = won ? "Marked as won." : "Marked as lost."; message = nil; Haptics.success(); await load() }
            catch { message = (error as? APIError)?.message ?? "Couldn't save that."; notice = nil; Haptics.error() }
        }
    }

    /// Approved payouts: the transfer is made outside the app, then recorded here with the bank's reference.
    var payCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Approved, ready to pay", systemImage: "banknote").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                ForEach(toPay) { p in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(Money(minor: p.amountMinor).display).vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                        if let why = p.purpose, !why.isEmpty { Text(why).vfont(13).foregroundStyle(VUI.textSecondary) }
                        Button("I made this transfer…") { paying = p }.vfont(14, .semibold).foregroundStyle(VUI.mint)
                            .accessibilityLabel("Record the transfer of \(Money(minor: p.amountMinor).display)")
                    }
                    if p.id != toPay.last?.id { Divider().overlay(.white.opacity(0.1)) }
                }
                Text("Vayora does not move this money. Make the transfer at your bank, then record it here.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
        }
    }

    var failedCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Refunds that failed", systemImage: "arrow.uturn.backward.circle").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                ForEach(failed) { r in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(Money(minor: r.amountMinor).display).vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                        if let why = r.failureReason, !why.isEmpty { Text(why).vfont(13).foregroundStyle(VUI.textSecondary) }
                        Button("Try again") { retry(r) }.vfont(14, .semibold).foregroundStyle(VUI.mint)
                            .accessibilityLabel("Retry the failed refund of \(Money(minor: r.amountMinor).display)")
                    }
                    if r.id != failed.last?.id { Divider().overlay(.white.opacity(0.1)) }
                }
            }
        }
    }

    func retry(_ r: FailedRefund) {
        guard let c = store.client else { return }
        Task {
            do { try await c.retryRefund(id: r.id); notice = "Refund sent again."; message = nil; Haptics.success(); await load() }
            catch { message = (error as? APIError)?.message ?? "Couldn't retry that."; notice = nil; Haptics.error() }
        }
    }

    /// Money that does not match up and needs a person: a settlement that does not add up, a payment the provider never confirmed.
    var casesCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                Label("Money to check", systemImage: "magnifyingglass").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                ForEach(cases) { c in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(c.title).vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                            Spacer()
                            if let d = c.diffMinor, d != 0 { Text((d < 0 ? "−" : "+") + Money(minor: abs(d)).display).vfont(13, .semibold).foregroundStyle(VUI.amber) }
                        }
                        if let e = c.explanation, !e.isEmpty { Text(e).vfont(13).foregroundStyle(VUI.textSecondary) }
                        if let who = c.assignee, !who.isEmpty { Text("With \(who)").vfont(12).foregroundStyle(VUI.textTertiary) }
                        HStack(spacing: 16) {
                            Button("Give to someone") { caseWork = CaseWork(item: c, resolving: false) }.accessibilityLabel("Assign the \(c.title) case to someone")
                            Button("Mark resolved") { caseWork = CaseWork(item: c, resolving: true) }.accessibilityLabel("Resolve the \(c.title) case")
                            if let sid = c.settlementID { Button("See the settlement") { settlementShown = sid }.accessibilityLabel("See the settlement behind the \(c.title) case") }
                        }.vfont(14, .semibold).foregroundStyle(VUI.mint)
                    }
                    if c.id != cases.last?.id { Divider().overlay(.white.opacity(0.1)) }
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
                            Button("Approve") { decide(d, approve: true) }.foregroundStyle(VUI.mint).accessibilityLabel("Approve reminder for \(d.invoiceNumber ?? "invoice")")
                            Button("Reject", role: .destructive) { decide(d, approve: false) }.accessibilityLabel("Reject reminder for \(d.invoiceNumber ?? "invoice")")
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
            disputes = (try? await c.openDisputes()) ?? []
            toPay = (try? await c.approvedPayouts()) ?? []
            failed = (try? await c.failedRefunds()) ?? []
            cases = (try? await c.openReconCases()) ?? []
            await store.refreshAttention()
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

/// A written note about a dispute, with an optional link to a document you keep elsewhere.
struct EvidenceSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    let dispute: DisputeInfo
    var onSaved: () -> Void
    @State private var note = ""
    @State private var link = ""
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.9)
            VStack(alignment: .leading, spacing: 14) {
                Text("Add evidence").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Text("\(Money(minor: dispute.amountMinor).display) · \(dispute.reason)").vfont(14).foregroundStyle(VUI.textSecondary)
                TextField("What this shows (delivered, signed, used…)", text: $note, axis: .vertical).lineLimit(3...6).foregroundStyle(VUI.textPrimary)
                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                TextField("Link to a document (optional, https)", text: $link).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled().foregroundStyle(VUI.textPrimary)
                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                Button(action: save) { Text("Save").vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 14).background(VUI.mint, in: Capsule()) }
                    .buttonStyle(PressScale()).disabled(note.trimmingCharacters(in: .whitespaces).isEmpty).opacity(note.trimmingCharacters(in: .whitespaces).isEmpty ? 0.5 : 1)
                Button("Cancel") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity)
                Spacer()
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.large])
    }

    func save() {
        guard let c = store.client else { return }
        let l = link.trimmingCharacters(in: .whitespaces)
        if !l.isEmpty, URL(string: l)?.scheme != "https" { message = "The link must start with https://"; Haptics.error(); return }
        Task {
            do { try await c.addDisputeEvidence(id: dispute.id, note: note.trimmingCharacters(in: .whitespaces), url: l.isEmpty ? nil : l); Haptics.success(); onSaved(); dismiss() }
            catch { message = (error as? APIError)?.message ?? "Couldn't save the evidence."; Haptics.error() }
        }
    }
}

/// Records that a payout's transfer was made, with the bank's reference (UTR). This posts to the books once and cannot be undone here.
struct MarkPaidSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    let payout: ApprovedPayout
    var onSaved: () -> Void
    @State private var ref = ""
    @State private var message: String?
    @State private var busy = false

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.95)
            VStack(alignment: .leading, spacing: 14) {
                Text("Record the transfer").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Text("\(Money(minor: payout.amountMinor).display)\(payout.purpose.map { " · \($0)" } ?? "")").vfont(14).foregroundStyle(VUI.textSecondary)
                TextField("Bank reference (UTR), 6 to 40 characters", text: $ref).textInputAutocapitalization(.characters).autocorrectionDisabled().foregroundStyle(VUI.textPrimary)
                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                Text("Only do this after the money has left your bank. It is written to your books once and cannot be undone from here.").vfont(12).foregroundStyle(VUI.amber)
                if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                Button(action: save) { Text(busy ? "Recording…" : "Record transfer").vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 14).background(VUI.mint, in: Capsule()) }
                    .buttonStyle(PressScale()).disabled(!BankRef.isValid(ref) || busy).opacity(BankRef.isValid(ref) && !busy ? 1 : 0.5)
                Button("Cancel") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity)
                Spacer()
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.medium])
    }

    func save() {
        guard let c = store.client, BankRef.isValid(ref), !busy else { return }
        busy = true
        Task {
            defer { busy = false }
            do { try await c.markPayoutPaid(id: payout.id, bankRef: ref); Haptics.success(); onSaved(); dismiss() }
            catch { message = (error as? APIError)?.message ?? "Couldn't record the transfer."; Haptics.error() }
        }
    }
}

struct CaseWork: Identifiable { let item: ReconCase; let resolving: Bool; var id: String { item.id + (resolving ? "r" : "a") } }

/// Gives a reconciliation case to someone, or closes it with a note of what was found.
struct CaseSheet: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    let work: CaseWork
    var onSaved: () -> Void
    @State private var text = ""
    @State private var message: String?
    @State private var busy = false

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.15)
            VStack(alignment: .leading, spacing: 14) {
                Text(work.resolving ? "Close this case" : "Give this case to someone").vfont(24, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Text(work.item.title).vfont(14).foregroundStyle(VUI.textSecondary)
                TextField(work.resolving ? "What you found or did" : "Their name", text: $text, axis: .vertical).lineLimit(work.resolving ? 3...6 : 1...1).foregroundStyle(VUI.textPrimary)
                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                if work.resolving { Text("This is kept with the case. Closing it does not change any money; it only says a person looked.").vfont(12).foregroundStyle(VUI.textTertiary) }
                if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                Button(action: save) { Text(busy ? "Saving…" : (work.resolving ? "Mark resolved" : "Give it")).vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 14).background(VUI.mint, in: Capsule()) }
                    .buttonStyle(PressScale()).disabled(!CaseNote.isUsable(text) || busy).opacity(CaseNote.isUsable(text) && !busy ? 1 : 0.5)
                Button("Cancel") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity)
                Spacer()
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.medium])
    }

    func save() {
        guard let c = store.client, CaseNote.isUsable(text), !busy else { return }
        busy = true
        Task {
            defer { busy = false }
            do {
                if work.resolving { try await c.resolveReconCase(id: work.item.id, resolution: text) } else { try await c.assignReconCase(id: work.item.id, to: text) }
                Haptics.success(); onSaved(); dismiss()
            } catch { message = (error as? APIError)?.message ?? "Couldn't save that."; Haptics.error() }
        }
    }
}

struct SettlementRef: Identifiable { let id: String }
