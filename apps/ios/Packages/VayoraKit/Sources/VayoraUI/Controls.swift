import SwiftUI

/// The decision logic of the slider, apart from the view so it can be tested.
public enum SlideOutcome: Equatable { case complete, reset }
public enum SlideModel {
    /// Release past 92% of the track completes; anything less springs back.
    public static func outcome(progress: Double) -> SlideOutcome { progress >= 0.92 ? .complete : .reset }
    /// Haptic tick index (one per 10% of travel), so ticks fire once per step, not per frame.
    public static func tickStep(progress: Double) -> Int { Int((max(0, min(1, progress)) * 10).rounded(.down)) }
}

/// Slide to pay: slow to start, impossible to tap by accident. `frozen` pins the thumb for gallery renders.
public struct SlideToPay: View {
    let label: String, brand: Brand, frozen: Double?, onComplete: () -> Void
    @State private var drag: CGFloat = 0
    @State private var lastTick = 0
    @State private var done = false
    private let thumb: CGFloat = 60

    public init(_ label: String, brand: Brand, frozen: Double? = nil, onComplete: @escaping () -> Void) {
        self.label = label; self.brand = brand; self.frozen = frozen; self.onComplete = onComplete
    }

    public var body: some View {
        GeometryReader { g in
            let travel = max(1, g.size.width - thumb - 8)
            let p = frozen ?? Double(min(1, max(0, drag / travel)))
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.08)).overlay(Capsule().strokeBorder(VUI.hairline))
                Capsule().fill(brand.gradient).frame(width: 4 + thumb + travel * CGFloat(p)).opacity(0.35 + 0.65 * p)
                Text(label).vfont(17, .semibold).foregroundStyle(VUI.textPrimary.opacity(1 - p * 1.4))
                    .frame(maxWidth: .infinity).padding(.leading, thumb * 0.6)
                Circle().fill(.white).frame(width: thumb, height: thumb)
                    .overlay(Image(systemName: done ? "checkmark" : "chevron.right.2").font(.system(size: 20, weight: .bold)).foregroundStyle(brand.deep))
                    .shadow(color: .black.opacity(0.35), radius: 8, y: 3)
                    .offset(x: 4 + travel * CGFloat(p))
                    .gesture(DragGesture().onChanged { v in
                        guard !done, frozen == nil else { return }
                        drag = min(travel, max(0, v.translation.width))
                        let step = SlideModel.tickStep(progress: Double(drag / travel))
                        if step != lastTick { lastTick = step; Haptics.tick() }
                    }.onEnded { _ in
                        guard !done, frozen == nil else { return }
                        if SlideModel.outcome(progress: Double(drag / travel)) == .complete {
                            withAnimation(VUI.snap) { drag = travel; done = true }
                            Haptics.thud(); onComplete()
                        } else { withAnimation(VUI.spring) { drag = 0 }; lastTick = 0 }
                    })
            }
        }
        .frame(height: thumb + 8)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label).accessibilityAddTraits(.isButton)
        .accessibilityAction { if !done { done = true; onComplete() } }
    }
}

public struct PinEntry: Equatable, Sendable {
    public let length: Int
    public private(set) var digits: [Int] = []
    public init(length: Int = 4) { self.length = length }
    public var code: String { digits.map(String.init).joined() }
    public var isFull: Bool { digits.count == length }
    public mutating func press(_ d: Int) { guard (0...9).contains(d), !isFull else { return }; digits.append(d) }
    public mutating func delete() { _ = digits.popLast() }
    public mutating func clear() { digits = [] }
}

public struct PinDots: View {
    let filled: Int, total: Int, shake: CGFloat, colour: Color
    public init(filled: Int, total: Int = 4, shake: CGFloat = 0, colour: Color = .white) { self.filled = filled; self.total = total; self.shake = shake; self.colour = colour }
    public var body: some View {
        HStack(spacing: 20) {
            ForEach(0..<total, id: \.self) { i in
                Circle().fill(i < filled ? colour : .white.opacity(0.16)).frame(width: 16, height: 16)
                    .scaleEffect(i < filled ? 1.15 : 1).animation(VUI.snap, value: filled)
            }
        }
        .offset(x: shake)
        .accessibilityElement(children: .ignore).accessibilityLabel("PIN, \(filled) of \(total) digits entered")
    }
}

public struct KeyPad: View {
    public struct Extra {
        let title: String?, icon: String?, label: String, action: () -> Void
        public init(title: String? = nil, icon: String? = nil, label: String, action: @escaping () -> Void) { self.title = title; self.icon = icon; self.label = label; self.action = action }
    }
    let onDigit: (Int) -> Void, onDelete: () -> Void, extraKey: Extra?
    public init(onDigit: @escaping (Int) -> Void, onDelete: @escaping () -> Void, extraKey: Extra? = nil) {
        self.onDigit = onDigit; self.onDelete = onDelete; self.extraKey = extraKey
    }
    public var body: some View {
        VStack(spacing: 12) {
            ForEach([[1, 2, 3], [4, 5, 6], [7, 8, 9]], id: \.self) { row in
                HStack(spacing: 12) { ForEach(row, id: \.self) { d in key(String(d)) { Haptics.tap(); onDigit(d) } } }
            }
            HStack(spacing: 12) {
                if let e = extraKey { key(e.title, icon: e.icon, label: e.label) { Haptics.tap(); e.action() } } else { Color.clear.frame(maxWidth: .infinity, minHeight: 64) }
                key("0") { Haptics.tap(); onDigit(0) }
                key(nil, icon: "delete.left", label: "Delete") { Haptics.tick(); onDelete() }
            }
        }
    }
    private func key(_ text: String?, icon: String? = nil, label: String? = nil, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Group {
                if let text { Text(text).vfont(30, .semibold) } else if let icon { Image(systemName: icon).font(.system(size: 22, weight: .semibold)) }
            }
            .foregroundStyle(VUI.textPrimary).frame(maxWidth: .infinity, minHeight: 64)
            .background(.white.opacity(0.07), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.hairline))
        }
        .buttonStyle(PressScale())
        .accessibilityLabel(label ?? text ?? "")
    }
}

public struct PressScale: ButtonStyle {
    public init() {}
    public func makeBody(configuration: Configuration) -> some View {
        configuration.label.scaleEffect(configuration.isPressed ? 0.94 : 1).brightness(configuration.isPressed ? 0.08 : 0).animation(VUI.snap, value: configuration.isPressed)
    }
}
