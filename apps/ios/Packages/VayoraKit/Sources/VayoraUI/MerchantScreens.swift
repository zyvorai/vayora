import SwiftUI
import VayoraKit

public struct PaymentRowModel: Identifiable, Equatable, Sendable {
    public enum State: Sendable { case succeeded, processing, refunded, partRefunded, failed }
    public let id: String, amount: Money, when: String, note: String, state: State
    public init(id: String, amount: Money, when: String, note: String = "", state: State) { self.id = id; self.amount = amount; self.when = when; self.note = note; self.state = state }
    var pill: (String, StatusPill.Kind) {
        switch state {
        case .succeeded: ("Paid", .success)
        case .processing: ("Waiting", .neutral)
        case .refunded: ("Refunded", .warn)
        case .partRefunded: ("Part refunded", .warn)
        case .failed: ("Failed", .danger)
        }
    }
}

/// Counter keypad: the amount is the screen.
public struct CollectKeypadScreen: View {
    let text: String, note: String, chips: [String], canContinue: Bool, title: String, actionLabel: String, actionIcon: String, brand: Brand
    let onDigit: (Int) -> Void, onPoint: () -> Void, onDelete: () -> Void, onChip: (String) -> Void, onShowQR: () -> Void
    public init(text: String, note: String, chips: [String], canContinue: Bool, title: String = "Collect", actionLabel: String = "Show QR", actionIcon: String = "qrcode", brand: Brand = .merchant,
                onDigit: @escaping (Int) -> Void = { _ in }, onPoint: @escaping () -> Void = {},
                onDelete: @escaping () -> Void = {}, onChip: @escaping (String) -> Void = { _ in }, onShowQR: @escaping () -> Void = {}) {
        self.text = text; self.note = note; self.chips = chips; self.canContinue = canContinue
        self.title = title; self.actionLabel = actionLabel; self.actionIcon = actionIcon; self.brand = brand
        self.onDigit = onDigit; self.onPoint = onPoint; self.onDelete = onDelete; self.onChip = onChip; self.onShowQR = onShowQR
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: brand, phase: 0.3)
            VStack(spacing: 16) {
                HStack { Text(title).font(VUI.display(28)).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
                Spacer(minLength: 0)
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text("₹").font(VUI.display(40, .semibold)).foregroundStyle(VUI.textSecondary)
                    Text(text.isEmpty ? "0" : text).font(VUI.display(84)).foregroundStyle(text.isEmpty ? VUI.textTertiary : VUI.textPrimary)
                        .minimumScaleFactor(0.4).lineLimit(1).contentTransition(.numericText())
                }
                .animation(VUI.snap, value: text).accessibilityElement(children: .ignore).accessibilityLabel("Amount \(text.isEmpty ? "zero" : text) rupees")
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(chips, id: \.self) { c in
                            Button { onChip(c) } label: {
                                Text(c).font(.system(size: 14, weight: .semibold, design: .rounded)).foregroundStyle(note == c ? VUI.ink : VUI.textPrimary)
                                    .padding(.horizontal, 14).padding(.vertical, 8).background(note == c ? AnyShapeStyle(brand.glow) : AnyShapeStyle(.white.opacity(0.10)), in: Capsule())
                            }.buttonStyle(.plain)
                        }
                    }.padding(.horizontal, 4)
                }.frame(height: 40)
                Spacer(minLength: 0)
                KeyPad(onDigit: onDigit, onDelete: onDelete, extraKey: KeyPad.Extra(title: ".", label: "Decimal point", action: onPoint))
                Button(action: onShowQR) {
                    Label(actionLabel, systemImage: actionIcon).font(VUI.display(19, .semibold)).foregroundStyle(canContinue ? VUI.ink : VUI.textTertiary)
                        .frame(maxWidth: .infinity).padding(.vertical, 18)
                        .background(canContinue ? AnyShapeStyle(LinearGradient(colors: [brand.glow, brand.primary], startPoint: .top, endPoint: .bottom)) : AnyShapeStyle(.white.opacity(0.08)), in: RoundedRectangle(cornerRadius: 26, style: .continuous))
                }.buttonStyle(PressScale()).disabled(!canContinue)
            }
            .padding(.horizontal, 20).padding(.bottom, 8)
        }
        .environment(\.colorScheme, .dark)
    }
}

/// The QR as the hero, with a live ring and an expiry arc. Customer mode strips everything else away.
public struct QRHeroScreen: View {
    let amount: Money, qr: Image?, expiry: Double, pulse: Double, customerMode: Bool, note: String, onCustomerMode: () -> Void, onCancel: () -> Void
    public init(amount: Money, qr: Image?, expiry: Double, pulse: Double, customerMode: Bool, note: String = "", onCustomerMode: @escaping () -> Void = {}, onCancel: @escaping () -> Void = {}) {
        self.amount = amount; self.qr = qr; self.expiry = expiry; self.pulse = pulse; self.customerMode = customerMode; self.note = note
        self.onCustomerMode = onCustomerMode; self.onCancel = onCancel
    }
    public var body: some View {
        ZStack {
            if customerMode { Color.white } else { AmbientBackground(brand: .merchant, phase: 0.55) }
            VStack(spacing: 22) {
                if !customerMode { HStack { Text("Waiting for payment").font(VUI.display(22)).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() } }
                Spacer(minLength: 0)
                AmountText(amount, size: customerMode ? 64 : 52).environment(\.colorScheme, customerMode ? .light : .dark)
                    .colorMultiply(customerMode ? .black : .white)
                ZStack {
                    RoundedRectangle(cornerRadius: 56, style: .continuous).stroke(VUI.emerald.opacity(0.3 * (1 - pulse)), lineWidth: 2).scaleEffect(1 + 0.10 * pulse).frame(width: 322, height: 322)
                    RoundedRectangle(cornerRadius: 34, style: .continuous).fill(.white).frame(width: 288, height: 288)
                        .shadow(color: VUI.emerald.opacity(customerMode ? 0.0 : 0.5), radius: 30)
                    if let qr { qr.interpolation(.none).resizable().scaledToFit().frame(width: 246, height: 246).accessibilityLabel("Payment QR code") }
                    else { Image(systemName: "qrcode").font(.system(size: 160)).foregroundStyle(.black) }
                    FrameProgress(expiry, cornerRadius: 46, lineWidth: 5, colour: VUI.emerald).frame(width: 308, height: 308)
                }
                if !note.isEmpty { Text(note).font(.system(size: 16, design: .rounded)).foregroundStyle(customerMode ? Color.black.opacity(0.6) : VUI.textSecondary) }
                HStack(spacing: 8) { Circle().fill(VUI.emerald).frame(width: 8, height: 8).opacity(0.4 + 0.6 * (1 - pulse)); Text("Scan with Vayora Pay to pay (test)").font(.system(size: 13, design: .rounded)) }
                    .foregroundStyle(customerMode ? Color.black.opacity(0.5) : VUI.textTertiary)
                Spacer(minLength: 0)
                HStack(spacing: 12) {
                    Button(action: onCustomerMode) { Label(customerMode ? "Exit customer view" : "Customer view", systemImage: customerMode ? "xmark" : "person.2.fill")
                        .font(.system(size: 16, weight: .semibold, design: .rounded)).foregroundStyle(customerMode ? Color.black : VUI.textPrimary).frame(maxWidth: .infinity).padding(.vertical, 16)
                        .background(customerMode ? Color.black.opacity(0.08) : .white.opacity(0.12), in: RoundedRectangle(cornerRadius: 22, style: .continuous)) }.buttonStyle(.plain)
                    if !customerMode { Button(action: onCancel) { Text("Cancel").font(.system(size: 16, weight: .semibold, design: .rounded)).foregroundStyle(VUI.coral).padding(.horizontal, 24).padding(.vertical, 16)
                        .background(VUI.coral.opacity(0.12), in: RoundedRectangle(cornerRadius: 22, style: .continuous)) }.buttonStyle(.plain) }
                }
            }
            .padding(24)
        }
        .environment(\.colorScheme, customerMode ? .light : .dark)
    }
}

/// Today at a glance, then the ledger of payments.
public struct MerchantTodayScreen: View {
    let collected: Money, count: Int, average: Money, trend: [Double], rows: [PaymentRowModel], scrolls: Bool, error: String?, onOpen: (PaymentRowModel) -> Void
    public init(collected: Money, count: Int, average: Money, trend: [Double], rows: [PaymentRowModel], scrolls: Bool = true, error: String? = nil, onOpen: @escaping (PaymentRowModel) -> Void = { _ in }) {
        self.collected = collected; self.count = count; self.average = average; self.trend = trend; self.rows = rows; self.scrolls = scrolls; self.error = error; self.onOpen = onOpen
    }
    public var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.12)
            if scrolls { ScrollView { content } } else { content.frame(maxHeight: .infinity, alignment: .top) }
        }
        .environment(\.colorScheme, .dark)
    }
    var content: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack { Text("Payments").font(VUI.display(28)).foregroundStyle(VUI.textPrimary); Spacer(); TestChip() }
            GlassCard {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Collected today").font(.system(size: 14, weight: .medium, design: .rounded)).foregroundStyle(VUI.textSecondary)
                    AmountText(collected, size: 52)
                    Sparkline(trend, colour: VUI.mint).frame(height: 56)
                    HStack {
                        stat("Payments", "\(count)"); Spacer(); stat("Average", average.display)
                    }
                }
            }
            if let error { Text(error).font(.system(size: 14, design: .rounded)).foregroundStyle(VUI.coral) }
            Text("Recent").font(VUI.display(20)).foregroundStyle(VUI.textPrimary)
            if rows.isEmpty && error == nil { Text("No payments yet. Create one on the Collect tab.").font(.system(size: 15, design: .rounded)).foregroundStyle(VUI.textSecondary) }
            ForEach(rows) { r in
              Button { onOpen(r) } label: {
                HStack(spacing: 14) {
                    Image(systemName: icon(r.state)).font(.system(size: 18, weight: .semibold)).foregroundStyle(tint(r.state))
                        .frame(width: 44, height: 44).background(tint(r.state).opacity(0.14), in: Circle())
                    VStack(alignment: .leading, spacing: 3) {
                        Text(r.amount.display).font(VUI.display(18, .semibold)).foregroundStyle(VUI.textPrimary)
                        Text(r.note.isEmpty ? r.when : "\(r.note) · \(r.when)").font(.system(size: 12.5, design: .rounded)).foregroundStyle(VUI.textTertiary)
                    }
                    Spacer()
                    StatusPill(r.pill.0, r.pill.1)
                }
                .padding(14).background(.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.hairline))
              }
              .buttonStyle(PressScale())
            }
        }
        .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
    }
    func stat(_ k: String, _ v: String) -> some View {
        VStack(alignment: .leading, spacing: 2) { Text(k).font(.system(size: 12.5, design: .rounded)).foregroundStyle(VUI.textTertiary); Text(v).font(VUI.display(18, .semibold)).foregroundStyle(VUI.textPrimary) }
    }
    func icon(_ s: PaymentRowModel.State) -> String { switch s { case .succeeded: "arrow.down.left"; case .processing: "hourglass"; case .refunded, .partRefunded: "arrow.uturn.backward"; case .failed: "xmark" } }
    func tint(_ s: PaymentRowModel.State) -> Color { switch s { case .succeeded: VUI.mint; case .processing: VUI.textSecondary; case .refunded, .partRefunded: VUI.amber; case .failed: VUI.coral } }
}
