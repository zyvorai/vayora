import SwiftUI
import VayoraKit

/// How a mandate reads on screen: a word and a colour, from the server's status.
public enum MandateStatusKind: Equatable, Sendable {
    case waiting, active, paused, revoked(by: String?), expired, declined

    public init(status: String, revokedBy: String? = nil) {
        switch status {
        case "active": self = .active
        case "paused": self = .paused
        case "revoked": self = .revoked(by: revokedBy)
        case "expired": self = .expired
        case "failed": self = .declined
        default: self = .waiting
        }
    }
    public var pill: (String, StatusPill.Kind) {
        switch self {
        case .waiting: ("Waiting for payer", .neutral)
        case .active: ("Active", .success)
        case .paused: ("Paused", .warn)
        case .revoked: ("Stopped", .danger)
        case .expired: ("Ended", .neutral)
        case .declined: ("Declined", .danger)
        }
    }
    public var isLive: Bool { switch self { case .waiting, .active, .paused: true; default: false } }
}

public struct MandateRowModel: Identifiable, Equatable, Sendable {
    public let id: String, name: String, line: String, status: MandateStatusKind
    public init(id: String, name: String, line: String, status: MandateStatusKind) { self.id = id; self.name = name; self.line = line; self.status = status }
}

public struct DebitRowModel: Identifiable, Equatable, Sendable {
    public let id: String, amount: Money, when: String, state: String
    public init(id: String, amount: Money, when: String, state: String) { self.id = id; self.amount = amount; self.when = when; self.state = state }
    var pill: (String, StatusPill.Kind) {
        switch state {
        case "captured": ("Taken", .success)
        case "failed": ("Failed", .danger)
        case "skipped": ("Cancelled", .neutral)
        case "initiating", "initiated": ("Sending", .warn)
        case "notified": ("Announced", .warn)
        default: ("Scheduled", .neutral)
        }
    }
}

// MARK: - Pay: approve

/// "Allow this merchant to take money on a schedule?" Slow to start, like paying.
public struct MandateReviewScreen: View {
    let merchant: String, payee: String, cap: Money, frequency: String, until: String?, frozen: Double?, onApprove: () -> Void
    public init(merchant: String, payee: String, cap: Money, frequency: String, until: String?, frozen: Double? = nil, onApprove: @escaping () -> Void = {}) {
        self.merchant = merchant; self.payee = payee; self.cap = cap; self.frequency = frequency; self.until = until; self.frozen = frozen; self.onApprove = onApprove
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.35)
            VStack(spacing: 16) {
                HStack { Spacer(); TestChip() }
                Spacer(minLength: 4)
                AvatarBadge(seed: payee, label: merchant.isEmpty ? payee : merchant, size: 84)
                Text("Let \(merchant.isEmpty ? payee : merchant) take money from you?").vfont(25, .bold).foregroundStyle(VUI.textPrimary).multilineTextAlignment(.center)
                VStack(spacing: 2) {
                    Text("Up to").vfont(14).foregroundStyle(VUI.textSecondary)
                    AmountText(cap, size: 54)
                    Text("each time").vfont(14).foregroundStyle(VUI.textSecondary)
                }.padding(.top, 4)
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        row("How often", frequency)
                        if let until { row("Until", until) }
                        row("Paid from", "Your test wallet")
                    }
                }
                Text("They can take this without you paying again. You can stop it any time from AutoPay.").vfont(13).foregroundStyle(VUI.textTertiary).multilineTextAlignment(.center).padding(.horizontal, 8)
                Spacer()
                SlideToPay("Slide to allow", brand: .pay, frozen: frozen, onComplete: onApprove)
                Text("Test mode. No real money moves.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
    func row(_ k: String, _ v: String) -> some View {
        HStack { Text(k).foregroundStyle(VUI.textSecondary); Spacer(); Text(v).foregroundStyle(VUI.textPrimary).multilineTextAlignment(.trailing) }.vfont(15)
    }
}

// MARK: - Pay: the list

public struct PayerMandateCardModel: Identifiable, Equatable, Sendable {
    public let id: String, merchant: String, payee: String, cap: Money, frequency: String, until: String?
    public let status: MandateStatusKind, taken: Money, recent: [DebitRowModel]
    public init(id: String, merchant: String, payee: String, cap: Money, frequency: String, until: String?, status: MandateStatusKind, taken: Money, recent: [DebitRowModel]) {
        self.id = id; self.merchant = merchant; self.payee = payee; self.cap = cap; self.frequency = frequency; self.until = until; self.status = status; self.taken = taken; self.recent = recent
    }
}

public struct PayerAutoPayList: View {
    let cards: [PayerMandateCardModel], onRevoke: (PayerMandateCardModel) -> Void
    public init(cards: [PayerMandateCardModel], onRevoke: @escaping (PayerMandateCardModel) -> Void = { _ in }) { self.cards = cards; self.onRevoke = onRevoke }
    public var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if cards.isEmpty {
                GlassCard {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("No AutoPay yet", systemImage: "arrow.triangle.2.circlepath").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                        Text("When a merchant asks to take money on a schedule, scan their AutoPay QR here. You approve once, and you can stop it any time.").vfont(14).foregroundStyle(VUI.textSecondary)
                    }
                }
            }
            ForEach(cards) { c in
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 12) {
                            AvatarBadge(seed: c.payee, label: c.merchant.isEmpty ? c.payee : c.merchant, size: 44)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(c.merchant.isEmpty ? c.payee : c.merchant).vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Text("Up to \(c.cap.display) · \(c.frequency)").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                            Spacer()
                            StatusPill(c.status.pill.0, c.status.pill.1)
                        }
                        HStack {
                            Text("Taken so far").foregroundStyle(VUI.textSecondary); Spacer()
                            Text(c.taken.display).foregroundStyle(VUI.textPrimary).fontWeight(.semibold)
                        }.vfont(15)
                        if let u = c.until { HStack { Text("Until").foregroundStyle(VUI.textSecondary); Spacer(); Text(u).foregroundStyle(VUI.textPrimary) }.vfont(15) }
                        ForEach(c.recent.prefix(3)) { d in
                            HStack { Text(d.when).foregroundStyle(VUI.textTertiary); Spacer(); Text(d.amount.display).foregroundStyle(VUI.textPrimary) }.vfont(13)
                        }
                        if c.status.isLive {
                            Button(role: .destructive) { onRevoke(c) } label: {
                                Text("Stop AutoPay").vfont(15, .semibold).foregroundStyle(VUI.coral).frame(maxWidth: .infinity).padding(.vertical, 12)
                                    .background(VUI.coral.opacity(0.14), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                            }.buttonStyle(PressScale())
                        }
                    }
                }
            }
            Text("You will not see upcoming debits here. A merchant is meant to tell you before each one; Vayora does not deliver that notice. Debits already taken appear above and in History.").vfont(12).foregroundStyle(VUI.textTertiary)
        }
    }
}

// MARK: - Merchant

public struct AutoPayListScreen: View {
    let rows: [MandateRowModel], scrolls: Bool, error: String?, onOpen: (MandateRowModel) -> Void, onNew: () -> Void
    public init(rows: [MandateRowModel], scrolls: Bool = true, error: String? = nil, onOpen: @escaping (MandateRowModel) -> Void = { _ in }, onNew: @escaping () -> Void = {}) {
        self.rows = rows; self.scrolls = scrolls; self.error = error; self.onOpen = onOpen; self.onNew = onNew
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.3)
            if scrolls { ScrollView { content } } else { content.frame(maxHeight: .infinity, alignment: .top) }
        }
        .environment(\.colorScheme, .dark)
    }
    var content: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("AutoPay").vfont(28, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                Spacer(); TestChip()
            }
            Button(action: onNew) {
                Label("New AutoPay", systemImage: "plus").vfont(18, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 16)
                    .background(LinearGradient(colors: [VUI.mint, VUI.emerald], startPoint: .top, endPoint: .bottom), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
            }.buttonStyle(PressScale())
            if let error { Text(error).vfont(14).foregroundStyle(VUI.coral) }
            if rows.isEmpty && error == nil {
                GlassCard { Text("Ask a customer to approve once, then take payments on a schedule. They are told before each debit and can stop it any time.").vfont(15).foregroundStyle(VUI.textSecondary) }
            }
            ForEach(rows) { r in
                Button { onOpen(r) } label: {
                    HStack(spacing: 14) {
                        AvatarBadge(seed: r.id, label: r.name, size: 46)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(r.name).vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                            Text(r.line).vfont(13).foregroundStyle(VUI.textTertiary)
                        }
                        Spacer()
                        StatusPill(r.status.pill.0, r.status.pill.1)
                    }
                    .padding(14).background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.hairline))
                }.buttonStyle(PressScale())
            }
        }
        .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
    }
}

/// The mandate's QR, then the moment it turns active. `approved` 0...1 drives the tick so a frame can be reviewed.
public struct MandateQRScreen: View {
    let customer: String, cap: Money, frequency: String, until: String, qr: Image?, status: MandateStatusKind, approved: Double, onDone: () -> Void
    public init(customer: String, cap: Money, frequency: String, until: String, qr: Image?, status: MandateStatusKind, approved: Double = 0, onDone: @escaping () -> Void = {}) {
        self.customer = customer; self.cap = cap; self.frequency = frequency; self.until = until; self.qr = qr; self.status = status; self.approved = approved; self.onDone = onDone
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.5)
            VStack(spacing: 16) {
                HStack { Text(status == .active ? "AutoPay is on" : "Ask \(customer) to scan").vfont(22, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
                Text("Up to \(cap.display) · \(frequency) · until \(until)").vfont(15).foregroundStyle(VUI.textSecondary).frame(maxWidth: .infinity, alignment: .leading)
                Spacer(minLength: 0)
                ZStack {
                    RoundedRectangle(cornerRadius: 34, style: .continuous).fill(.white).frame(width: 288, height: 288).opacity(status == .active ? 0.15 : 1)
                    if status == .active {
                        CheckmarkDraw(progress: approved, colour: VUI.mint).frame(width: 150, height: 150)
                    } else if let qr {
                        qr.interpolation(.none).resizable().scaledToFit().frame(width: 246, height: 246).accessibilityLabel("AutoPay approval QR code")
                    } else { Image(systemName: "qrcode").font(.system(size: 160)).foregroundStyle(.black) }
                }
                HStack(spacing: 8) {
                    Circle().fill(status == .active ? VUI.mint : VUI.amber).frame(width: 8, height: 8)
                    Text(status == .active ? "\(customer) approved it" : "Scan with Vayora Pay to approve (test)").vfont(14)
                }.foregroundStyle(VUI.textSecondary)
                Spacer(minLength: 0)
                Button(action: onDone) {
                    Text(status == .active ? "Open AutoPay" : "Close").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                        .background(.white.opacity(0.14), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                }.buttonStyle(PressScale())
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}

public struct MandateDetailScreen: View {
    let name: String, cap: Money, line: String, status: MandateStatusKind, note: String?, debits: [DebitRowModel], scrolls: Bool
    let onSchedule: () -> Void, onRunNow: () -> Void, onPauseResume: () -> Void, onRevoke: () -> Void
    public init(name: String, cap: Money, line: String, status: MandateStatusKind, note: String? = nil, debits: [DebitRowModel], scrolls: Bool = true,
                onSchedule: @escaping () -> Void = {}, onRunNow: @escaping () -> Void = {}, onPauseResume: @escaping () -> Void = {}, onRevoke: @escaping () -> Void = {}) {
        self.name = name; self.cap = cap; self.line = line; self.status = status; self.note = note; self.debits = debits; self.scrolls = scrolls
        self.onSchedule = onSchedule; self.onRunNow = onRunNow; self.onPauseResume = onPauseResume; self.onRevoke = onRevoke
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.4)
            if scrolls { ScrollView { content } } else { content.frame(maxHeight: .infinity, alignment: .top) }
        }
        .environment(\.colorScheme, .dark)
    }
    var content: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 14) {
                AvatarBadge(seed: name, label: name, size: 56)
                VStack(alignment: .leading, spacing: 4) {
                    Text(name).vfont(24, .bold).foregroundStyle(VUI.textPrimary)
                    StatusPill(status.pill.0, status.pill.1)
                }
                Spacer()
            }
            GlassCard {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Up to").vfont(14).foregroundStyle(VUI.textSecondary)
                    AmountText(cap, size: 46)
                    Text(line).vfont(14).foregroundStyle(VUI.textSecondary)
                }
            }
            if let note { Text(note).vfont(13).foregroundStyle(VUI.amber) }
            if status == .active || status == .paused {
                action("Schedule a debit", "calendar.badge.plus", VUI.mint, onSchedule)
                HStack(spacing: 10) {
                    action(status == .paused ? "Resume" : "Pause", status == .paused ? "play.fill" : "pause.fill", VUI.textPrimary, onPauseResume)
                    action("Stop", "xmark.circle", VUI.coral, onRevoke)
                }
                action("Run now (sandbox)", "bolt.fill", VUI.amber, onRunNow)
            } else if status.isLive {
                action("Stop", "xmark.circle", VUI.coral, onRevoke)
            }
            Text("Debits").vfont(20, .bold).foregroundStyle(VUI.textPrimary)
            if debits.isEmpty { Text("None scheduled. A debit needs the payer to be told ahead of time, so it cannot be due today unless the server's notice window is zero.").vfont(14).foregroundStyle(VUI.textSecondary) }
            ForEach(debits) { d in
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(d.amount.display).vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                        Text(d.when).vfont(13).foregroundStyle(VUI.textTertiary)
                    }
                    Spacer()
                    StatusPill(d.pill.0, d.pill.1)
                }
                .padding(14).background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(VUI.hairline))
            }
            Text("Zyvor emits the pre-debit notice; it does not message the payer. Tell them before each debit.").vfont(12).foregroundStyle(VUI.textTertiary)
        }
        .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
    }
    func action(_ title: String, _ icon: String, _ colour: Color, _ run: @escaping () -> Void) -> some View {
        Button(action: { Haptics.tap(); run() }) {
            Label(title, systemImage: icon).vfont(16, .semibold).foregroundStyle(colour).frame(maxWidth: .infinity).padding(.vertical, 14)
                .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(VUI.hairline))
        }.buttonStyle(PressScale())
    }
}

/// Step one of a new AutoPay: who, how often, for how long. The cap comes next, on the keypad.
public struct NewAutoPayDetailsScreen: View {
    @Binding var name: String
    @Binding var frequency: MandateFrequency
    @Binding var months: Int
    let canContinue: Bool, onNext: () -> Void
    public init(name: Binding<String>, frequency: Binding<MandateFrequency>, months: Binding<Int>, canContinue: Bool, onNext: @escaping () -> Void = {}) {
        self._name = name; self._frequency = frequency; self._months = months; self.canContinue = canContinue; self.onNext = onNext
    }
    static let common: [MandateFrequency] = [.weekly, .monthly, .quarterly, .yearly, .asPresented]
    @ViewBuilder var nameField: some View {
        #if os(iOS)
        TextField("Name", text: $name).textInputAutocapitalization(.words)
        #else
        TextField("Name", text: $name)
        #endif
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.6)
            VStack(alignment: .leading, spacing: 18) {
                HStack { Text("New AutoPay").vfont(26, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
                GlassCard {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("Customer").vfont(14, .medium).foregroundStyle(VUI.textSecondary)
                        nameField.foregroundStyle(VUI.textPrimary)
                            .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    }
                }
                Text("How often").vfont(14, .medium).foregroundStyle(VUI.textSecondary)
                chips(Self.common.map { ($0.title, $0 == frequency) }) { i in frequency = Self.common[i] }
                Text("For how long").vfont(14, .medium).foregroundStyle(VUI.textSecondary)
                chips([3, 6, 12, 24].map { ("\($0) months", $0 == months) }) { i in months = [3, 6, 12, 24][i] }
                Spacer()
                Button(action: onNext) {
                    Text("Next: set the limit").vfont(18, .semibold).foregroundStyle(canContinue ? VUI.ink : VUI.textTertiary).frame(maxWidth: .infinity).padding(.vertical, 18)
                        .background(canContinue ? AnyShapeStyle(LinearGradient(colors: [VUI.mint, VUI.emerald], startPoint: .top, endPoint: .bottom)) : AnyShapeStyle(.white.opacity(0.08)), in: RoundedRectangle(cornerRadius: 26, style: .continuous))
                }.buttonStyle(PressScale()).disabled(!canContinue)
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
    func chips(_ items: [(String, Bool)], _ pick: @escaping (Int) -> Void) -> some View {
        FlowLayout(spacing: 8) {
            ForEach(Array(items.enumerated()), id: \.offset) { i, it in
                Button { Haptics.tick(); pick(i) } label: {
                    Text(it.0).vfont(15, .semibold).foregroundStyle(it.1 ? VUI.ink : VUI.textPrimary).padding(.horizontal, 14).padding(.vertical, 9)
                        .background(it.1 ? AnyShapeStyle(VUI.mint) : AnyShapeStyle(.white.opacity(0.10)), in: Capsule())
                }.buttonStyle(.plain)
            }
        }
    }
}

/// Chips that wrap onto new lines.
struct FlowLayout: Layout {
    var spacing: CGFloat = 8
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let w = proposal.width ?? 320
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x + sz.width > w { x = 0; y += rowH + spacing; rowH = 0 }
            x += sz.width + spacing; rowH = max(rowH, sz.height)
        }
        return CGSize(width: w, height: y + rowH)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, rowH: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x + sz.width > bounds.maxX { x = bounds.minX; y += rowH + spacing; rowH = 0 }
            s.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sz))
            x += sz.width + spacing; rowH = max(rowH, sz.height)
        }
    }
}
