import SwiftUI

/// A slow-drifting colour field on near-black. Pass `phase` to freeze it (gallery, Reduce Motion).
public struct AmbientBackground: View {
    let brand: Brand
    let phase: Double?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(brand: Brand, phase: Double? = nil) { self.brand = brand; self.phase = phase }

    public var body: some View {
        ZStack {
            VUI.ink
            if let phase { field(phase) }
            else if reduceMotion { field(0.25) }
            else {
                TimelineView(.animation(minimumInterval: 1.0 / 30)) { ctx in
                    field(ctx.date.timeIntervalSinceReferenceDate / 14)
                }
            }
            // keeps text legible wherever the colour lands
            LinearGradient(colors: [.clear, VUI.ink.opacity(0.65)], startPoint: .center, endPoint: .bottom)
        }
        .ignoresSafeArea()
    }

    private func field(_ t: Double) -> some View {
        GeometryReader { g in
            let w = g.size.width, h = g.size.height
            ZStack {
                blob(brand.primary, size: w * 1.05, x: w * (0.25 + 0.18 * sin(t * 6.28)), y: h * (0.12 + 0.05 * cos(t * 6.28)), o: 0.85)
                blob(brand.glow, size: w * 0.85, x: w * (0.85 - 0.16 * cos(t * 6.28 + 1)), y: h * (0.30 + 0.06 * sin(t * 6.28 + 2)), o: 0.55)
                blob(brand.deep, size: w * 1.1, x: w * (0.5 + 0.2 * sin(t * 6.28 + 3)), y: h * (0.68 + 0.05 * cos(t * 6.28)), o: 0.9)
            }
            .blur(radius: 70)
        }
    }

    private func blob(_ c: Color, size: CGFloat, x: CGFloat, y: CGFloat, o: Double) -> some View {
        Circle().fill(c.opacity(o)).frame(width: size, height: size).position(x: x, y: y)
    }
}

/// Frosted card. On iOS 26 and later it is real Liquid Glass; before that, a translucent fill with a lit top edge
/// and a hairline, which also renders the same everywhere (the Mac gallery included).
public struct GlassCard<Content: View>: View {
    let radius: CGFloat
    let content: Content
    public init(radius: CGFloat = VUI.Radius.card, @ViewBuilder _ content: () -> Content) { self.radius = radius; self.content = content() }
    public var body: some View {
        let padded = content.padding(VUI.Space.m + 4).frame(maxWidth: .infinity, alignment: .leading)
        if #available(iOS 26, *), VUI.useLiquidGlass {
            #if os(iOS)
            padded.glassEffect(.regular, in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            #else
            fallback(padded)
            #endif
        } else {
            fallback(padded)
        }
    }
    private func fallback<V: View>(_ v: V) -> some View {
        v.background {
            RoundedRectangle(cornerRadius: radius, style: .continuous)
                .fill(LinearGradient(colors: [.white.opacity(0.16), .white.opacity(0.06)], startPoint: .top, endPoint: .bottom))
                .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous)
                    .strokeBorder(LinearGradient(colors: [.white.opacity(0.45), .white.opacity(0.06)], startPoint: .top, endPoint: .bottom), lineWidth: 1))
        }
    }
}
