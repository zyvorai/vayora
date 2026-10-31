import SwiftUI

/// Design tokens for both apps. Dark-first: the canvas is near-black so colour and motion carry the screen.
public enum VUI {
    public static let ink = Color(red: 0.027, green: 0.027, blue: 0.043)
    public static let inkRaised = Color(red: 0.075, green: 0.075, blue: 0.105)
    public static let hairline = Color.white.opacity(0.10)
    public static let textPrimary = Color.white
    public static let textSecondary = Color.white.opacity(0.62)
    public static let textTertiary = Color.white.opacity(0.38)

    public static let indigo = Color(red: 0.42, green: 0.34, blue: 1.0)
    public static let indigoDeep = Color(red: 0.16, green: 0.10, blue: 0.55)
    public static let violet = Color(red: 0.72, green: 0.38, blue: 1.0)
    public static let emerald = Color(red: 0.10, green: 0.82, blue: 0.58)
    public static let emeraldDeep = Color(red: 0.02, green: 0.36, blue: 0.27)
    public static let mint = Color(red: 0.52, green: 1.0, blue: 0.82)
    public static let amber = Color(red: 1.0, green: 0.72, blue: 0.20)
    public static let coral = Color(red: 1.0, green: 0.34, blue: 0.40)

    /// Real Liquid Glass on iOS 26+. Flip to false if it ever reads badly over the colour field.
    public static let useLiquidGlass = true

    public enum Radius { public static let card: CGFloat = 28, control: CGFloat = 20, pill: CGFloat = 999 }
    public enum Space { public static let s: CGFloat = 8, m: CGFloat = 16, l: CGFloat = 24, xl: CGFloat = 32 }

    /// One spring for everything that moves, so the apps feel like one object.
    public static let spring = Animation.spring(response: 0.42, dampingFraction: 0.78)
    public static let snap = Animation.spring(response: 0.28, dampingFraction: 0.86)

    public static func display(_ size: CGFloat, _ weight: Font.Weight = .bold) -> Font { .system(size: size, weight: weight, design: .rounded) }
}

public struct Brand: Sendable {
    public let primary: Color, deep: Color, glow: Color
    public static let pay = Brand(primary: VUI.indigo, deep: VUI.indigoDeep, glow: VUI.violet)
    public static let merchant = Brand(primary: VUI.emerald, deep: VUI.emeraldDeep, glow: VUI.mint)
    public var gradient: LinearGradient { LinearGradient(colors: [primary, deep], startPoint: .topLeading, endPoint: .bottomTrailing) }
}

/// A rounded font that follows the user's text size. Plain `.system(size:)` ignores Dynamic Type; this scales with it.
struct ScaledFont: ViewModifier {
    @ScaledMetric private var size: CGFloat
    let weight: Font.Weight
    init(_ size: CGFloat, _ weight: Font.Weight) { _size = ScaledMetric(wrappedValue: size, relativeTo: size >= 28 ? .largeTitle : .body); self.weight = weight }
    func body(content: Content) -> some View { content.font(.system(size: size, weight: weight, design: .rounded)) }
}

public extension View {
    /// Rounded text at `size` points at the default setting, scaling with Dynamic Type.
    func vfont(_ size: CGFloat, _ weight: Font.Weight = .regular) -> some View { modifier(ScaledFont(size, weight)) }
}
