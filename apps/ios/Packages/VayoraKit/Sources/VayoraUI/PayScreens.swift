import SwiftUI
import VayoraKit

public struct TxnRowModel: Identifiable, Equatable, Sendable {
    public let id: String, name: String, payee: String, amount: Money, when: String, ok: Bool
    public init(id: String, name: String, payee: String, amount: Money, when: String, ok: Bool) {
        self.id = id; self.name = name; self.payee = payee; self.amount = amount; self.when = when; self.ok = ok
    }
}

public struct TestChip: View {
    public init() {}
    public var body: some View {
        Label("Test mode", systemImage: "flask.fill").vfont(11.5, .bold).kerning(0.4)
            .padding(.horizontal, 10).padding(.vertical, 5)
            .foregroundStyle(VUI.amber).background(VUI.amber.opacity(0.14), in: Capsule())
            .overlay(Capsule().strokeBorder(VUI.amber.opacity(0.3), lineWidth: 0.5))
            .accessibilityLabel("Test mode. No real money moves.")
    }
}

/// The pay home: wallet on glass, one huge scan action, a week at a glance, recent payments.
public struct PayHomeScreen: View {
    let balance: Money, spentThisMonth: Money, week: [Double], recent: [TxnRowModel]
    let phase: Double?, scrolls: Bool, summary: [(category: SpendCategory, minor: Int64)], hideBalance: Bool
    let onScan: () -> Void, onPaste: () -> Void, onSend: () -> Void, onSplit: () -> Void, onOpen: (TxnRowModel) -> Void
    public init(balance: Money, spentThisMonth: Money, week: [Double], recent: [TxnRowModel], phase: Double? = nil, scrolls: Bool = true,
                summary: [(category: SpendCategory, minor: Int64)] = [], hideBalance: Bool = false,
                onScan: @escaping () -> Void = {}, onPaste: @escaping () -> Void = {}, onSend: @escaping () -> Void = {}, onSplit: @escaping () -> Void = {},
                onOpen: @escaping (TxnRowModel) -> Void = { _ in }) {
        self.hideBalance = hideBalance; self.summary = summary; self.onSend = onSend; self.onSplit = onSplit
        self.balance = balance; self.spentThisMonth = spentThisMonth; self.week = week; self.recent = recent; self.phase = phase; self.scrolls = scrolls
        self.onScan = onScan; self.onPaste = onPaste; self.onOpen = onOpen
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: phase)
            if scrolls { ScrollView { content } } else { content.frame(maxHeight: .infinity, alignment: .top) }
        }
        .environment(\.colorScheme, .dark)
    }
    var content: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack { Text("Vayora Pay").vfont(28, .bold).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
            wallet
            scan
            HStack(spacing: 10) {
                quick("Send", "paperplane.fill", onSend)
                quick("Split", "divide.circle.fill", onSplit)
                quick("Paste link", "link", onPaste)
            }
            if !summary.isEmpty { MonthSummaryCard(items: summary, total: spentThisMonth) }
            insights
            recents
        }
        .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
    }
    func quick(_ title: String, _ icon: String, _ run: @escaping () -> Void) -> some View {
        Button(action: { Haptics.tap(); run() }) {
            VStack(spacing: 6) {
                Image(systemName: icon).font(.system(size: 20, weight: .semibold)).foregroundStyle(VUI.violet)
                Text(title).vfont(13, .semibold).foregroundStyle(VUI.textPrimary)
            }
            .frame(maxWidth: .infinity).padding(.vertical, 14)
            .background(.white.opacity(0.07), in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(VUI.hairline))
        }
        .buttonStyle(PressScale())
    }
    var wallet: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 14) {
                HStack { Text("Test balance").vfont(14, .medium).foregroundStyle(VUI.textSecondary); Spacer(); Image(systemName: "wave.3.right").foregroundStyle(VUI.textTertiary) }
                if hideBalance { Text("₹ ••••••").vfont(44, .bold).foregroundStyle(VUI.textPrimary) } else { AmountText(balance, size: 52) }
                HStack(spacing: 6) { Image(systemName: "iphone").font(.caption); Text("Held on this phone, not in a bank").vfont(12.5) }.foregroundStyle(VUI.textTertiary)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(hideBalance ? "Test balance hidden. Held on this phone, not in a bank." : "Test balance \(balance.display). Held on this phone, not in a bank.")
    }
    var scan: some View {
        Button(action: { Haptics.thud(); onScan() }) {
            HStack(spacing: 16) {
                ZStack { Circle().fill(.white.opacity(0.18)); Image(systemName: "qrcode.viewfinder").font(.system(size: 30, weight: .semibold)) }.frame(width: 62, height: 62)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Scan & pay").vfont(24, .bold)
                    Text("Aim at a Vayora Merchant QR").vfont(14).opacity(0.8)
                }
                Spacer()
                Image(systemName: "arrow.right").font(.system(size: 18, weight: .bold)).opacity(0.85)
            }
            .foregroundStyle(.white).padding(18)
            .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 30, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 30, style: .continuous).strokeBorder(.white.opacity(0.3), lineWidth: 1))
            .shadow(color: VUI.indigo.opacity(0.55), radius: 26, y: 12)
        }
        .buttonStyle(PressScale()).accessibilityHint("Opens the camera to scan a payment QR code")
    }
    var insights: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    Text("This week").vfont(14, .medium).foregroundStyle(VUI.textSecondary)
                    Spacer()
                    Text("Month \(spentThisMonth.display)").vfont(13, .semibold).foregroundStyle(VUI.textSecondary)
                }
                Sparkline(week, colour: VUI.violet).frame(height: 64)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Spending this week and month. This month \(spentThisMonth.display).")
    }
    var recents: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Recent").vfont(20, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
            if recent.isEmpty {
                GlassCard { Text("No payments yet. Scan a Vayora Merchant QR to make one.").vfont(15).foregroundStyle(VUI.textSecondary) }
            } else {
                ForEach(recent) { r in
                    Button { onOpen(r) } label: { TxnRowView(model: r) }.buttonStyle(PressScale())
                }
            }
        }
    }
}

/// One payment in a list: avatar, who, when, how much, and a status pill.
public struct TxnRowView: View {
    let model: TxnRowModel
    public init(model: TxnRowModel) { self.model = model }
    public var body: some View {
        HStack(spacing: 14) {
            AvatarBadge(seed: model.payee, label: model.name.isEmpty ? model.payee : model.name, size: 46)
            VStack(alignment: .leading, spacing: 3) {
                Text(model.name.isEmpty ? model.payee : model.name).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                Text(model.when).vfont(12.5).foregroundStyle(VUI.textTertiary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 4) {
                Text(model.amount.display).vfont(16, .semibold).foregroundStyle(VUI.textPrimary)
                StatusPill(model.ok ? "Paid" : "Failed", model.ok ? .success : .danger)
            }
        }
        .padding(14).background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.hairline))
        .accessibilityElement(children: .combine)
    }
}

/// Review before paying: who, how much, then slide.
public struct PayReviewScreen: View {
    let name: String, payee: String, amount: Money, balance: Money, frozen: Double?, onPay: () -> Void
    public init(name: String, payee: String, amount: Money, balance: Money, frozen: Double? = nil, onPay: @escaping () -> Void = {}) {
        self.name = name; self.payee = payee; self.amount = amount; self.balance = balance; self.frozen = frozen; self.onPay = onPay
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.2)
            VStack(spacing: 18) {
                HStack { Spacer(); TestChip() }
                Spacer(minLength: 8)
                AvatarBadge(seed: payee, label: name.isEmpty ? payee : name, size: 92)
                Text(name.isEmpty ? payee : name).vfont(26, .bold).foregroundStyle(VUI.textPrimary)
                Text(payee).vfont(14).foregroundStyle(VUI.textTertiary)
                AmountText(amount, size: 64).padding(.top, 10)
                HStack(spacing: 6) { Image(systemName: "wallet.pass").font(.caption); Text("Test balance \(balance.display)").vfont(13) }
                    .foregroundStyle(VUI.textSecondary).padding(.horizontal, 12).padding(.vertical, 7).background(.white.opacity(0.07), in: Capsule())
                Spacer()
                SlideToPay("Slide to pay", brand: .pay, frozen: frozen, onComplete: onPay)
                Text("No real money moves.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}

public struct PinScreen: View {
    let filled: Int, error: Bool, shake: CGFloat, onDigit: (Int) -> Void, onDelete: () -> Void, onBiometric: (() -> Void)?
    public init(filled: Int, error: Bool = false, shake: CGFloat = 0, onDigit: @escaping (Int) -> Void = { _ in }, onDelete: @escaping () -> Void = {}, onBiometric: (() -> Void)? = nil) {
        self.filled = filled; self.error = error; self.shake = shake; self.onDigit = onDigit; self.onDelete = onDelete; self.onBiometric = onBiometric
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.55)
            VStack(spacing: 26) {
                Spacer(minLength: 20)
                Image(systemName: error ? "lock.trianglebadge.exclamationmark.fill" : "lock.fill").font(.system(size: 34)).foregroundStyle(error ? VUI.coral : VUI.textPrimary)
                Text(error ? "That PIN is wrong" : "Enter your test PIN").vfont(24, .bold).foregroundStyle(VUI.textPrimary)
                PinDots(filled: filled, shake: shake, colour: error ? VUI.coral : .white)
                Text("A local test PIN, not a UPI PIN. Default 1234.").vfont(13).foregroundStyle(VUI.textTertiary)
                Spacer()
                KeyPad(onDigit: onDigit, onDelete: onDelete, extraKey: onBiometric.map { f in KeyPad.Extra(icon: "faceid", label: "Use Face ID", action: f) })
            }
            .padding(.horizontal, 24).padding(.bottom, 16)
        }
        .environment(\.colorScheme, .dark)
    }
}

public enum ResultKind: Equatable, Sendable { case paid, failed(String), checking }

public struct ResultScreen: View {
    let kind: ResultKind, amount: Money, to: String, progress: Double, brand: Brand, paidTitle: String, doneLabel: String, footnote: String?, onDone: () -> Void
    public init(kind: ResultKind, amount: Money, to: String, progress: Double, brand: Brand = .pay, paidTitle: String = "Paid", doneLabel: String = "Done", footnote: String? = nil, onDone: @escaping () -> Void = {}) {
        self.footnote = footnote; self.kind = kind; self.amount = amount; self.to = to; self.progress = progress; self.brand = brand; self.paidTitle = paidTitle; self.doneLabel = doneLabel; self.onDone = onDone
    }
    var colour: Color { switch kind { case .paid: VUI.mint; case .failed: VUI.coral; case .checking: VUI.amber } }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: kind == .paid ? brand : Brand(primary: colour.opacity(0.7), deep: VUI.inkRaised, glow: colour.opacity(0.5)), phase: 0.4)
            VStack(spacing: 16) {
                Spacer()
                ZStack {
                    switch kind {
                    case .paid:
                        CheckmarkDraw(progress: progress, colour: colour).frame(width: 150, height: 150)
                        ParticleBurst(progress: max(0, (progress - 0.5) * 2), colours: [VUI.mint, brand.glow, VUI.amber, .white, brand.primary]).frame(width: 340, height: 340)
                    case .failed:
                        Circle().fill(colour.opacity(0.14)).frame(width: 150, height: 150)
                        Image(systemName: "xmark").font(.system(size: 56, weight: .bold)).foregroundStyle(colour)
                    case .checking:
                        Circle().fill(colour.opacity(0.14)).frame(width: 150, height: 150)
                        Image(systemName: "hourglass").font(.system(size: 52, weight: .semibold)).foregroundStyle(colour)
                    }
                }.frame(height: 170)
                Text(title).vfont(30, .bold).foregroundStyle(VUI.textPrimary)
                if kind == .paid || kind == .checking { AmountText(amount, size: 48) }
                Text(detail).vfont(16).foregroundStyle(VUI.textSecondary).multilineTextAlignment(.center).padding(.horizontal, 24)
                if let footnote, !footnote.isEmpty { Text(footnote).font(.system(size: 11, design: .monospaced)).foregroundStyle(VUI.textTertiary).multilineTextAlignment(.center).padding(.horizontal, 24).lineLimit(5).textSelection(.enabled) }
                Spacer()
                Button(action: onDone) { Text(doneLabel).vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                    .background(.white.opacity(0.14), in: RoundedRectangle(cornerRadius: 22, style: .continuous)) }.buttonStyle(PressScale())
            }
            .padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
    var title: String { switch kind { case .paid: paidTitle; case .failed: "Payment failed"; case .checking: "Checking…" } }
    var detail: String {
        switch kind {
        case .paid: "to \(to)"
        case .failed(let why): why
        case .checking: "We couldn't confirm this payment. Don't pay again: ask the merchant if it arrived."
        }
    }
}
