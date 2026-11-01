import SwiftUI
import VayoraKit

/// Money as a headline: rupees large, paise smaller and aligned to the baseline. Digits roll when the value changes.
public struct AmountText: View {
    let money: Money
    let size: CGFloat
    public init(_ money: Money, size: CGFloat = 56) { self.money = money; self.size = size }
    public var body: some View {
        let parts = money.display.split(separator: ".", maxSplits: 1).map(String.init)
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Text(parts[0]).vfont(size, .bold).contentTransition(.numericText())
            if parts.count > 1 { Text("." + parts[1]).vfont(size * 0.5, .semibold).foregroundStyle(VUI.textSecondary).contentTransition(.numericText()) }
        }
        .foregroundStyle(VUI.textPrimary)
        .minimumScaleFactor(0.5).lineLimit(1)
        .animation(VUI.spring, value: money.minor)
        .accessibilityLabel(money.display)
    }
}

public struct StatusPill: View {
    public enum Kind { case success, warn, danger, neutral }
    let text: String, kind: Kind
    public init(_ text: String, _ kind: Kind) { self.text = text; self.kind = kind }
    var colour: Color { switch kind { case .success: VUI.mint; case .warn: VUI.amber; case .danger: VUI.coral; case .neutral: VUI.textSecondary } }
    public var body: some View {
        Text(text.uppercased()).vfont(10.5, .bold).kerning(0.6)
            .padding(.horizontal, 9).padding(.vertical, 4)
            .foregroundStyle(colour).background(colour.opacity(0.16), in: Capsule())
            .overlay(Capsule().strokeBorder(colour.opacity(0.28), lineWidth: 0.5))
    }
}

/// Initials on a colour derived from a stable hash, so the same payee always looks the same.
public struct AvatarBadge: View {
    let seed: String, label: String, size: CGFloat
    public init(seed: String, label: String, size: CGFloat = 64) { self.seed = seed; self.label = label; self.size = size }
    static func hue(_ s: String) -> Double {
        var h: UInt64 = 1469598103934665603
        for b in s.utf8 { h = (h ^ UInt64(b)) &* 1099511628211 }
        return Double(h % 360) / 360
    }
    var initials: String {
        let words = label.split(whereSeparator: { !$0.isLetter && !$0.isNumber }).prefix(2)
        let s = words.compactMap { $0.first }.map(String.init).joined().uppercased()
        return s.isEmpty ? "?" : s
    }
    public var body: some View {
        let h = Self.hue(seed)
        Text(initials).vfont(size * 0.38, .bold).foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(LinearGradient(colors: [Color(hue: h, saturation: 0.55, brightness: 0.95), Color(hue: (h + 0.08).truncatingRemainder(dividingBy: 1), saturation: 0.75, brightness: 0.62)], startPoint: .topLeading, endPoint: .bottomTrailing), in: Circle())
            .overlay(Circle().strokeBorder(.white.opacity(0.35), lineWidth: 1))
            .accessibilityHidden(true)
    }
}

public struct RingProgress: View {
    let progress: Double, lineWidth: CGFloat, colour: Color
    public init(_ progress: Double, lineWidth: CGFloat = 6, colour: Color) { self.progress = progress; self.lineWidth = lineWidth; self.colour = colour }
    public var body: some View {
        ZStack {
            Circle().stroke(colour.opacity(0.18), lineWidth: lineWidth)
            Circle().trim(from: 0, to: max(0, min(1, progress))).stroke(colour, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)).rotationEffect(.degrees(-90))
        }
    }
}

/// Like `RingProgress`, but follows a rounded square: the expiry arc hugs the QR card instead of cutting its corners.
public struct FrameProgress: View {
    let progress: Double, cornerRadius: CGFloat, lineWidth: CGFloat, colour: Color
    public init(_ progress: Double, cornerRadius: CGFloat, lineWidth: CGFloat = 5, colour: Color) { self.progress = progress; self.cornerRadius = cornerRadius; self.lineWidth = lineWidth; self.colour = colour }
    public var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous).stroke(colour.opacity(0.18), lineWidth: lineWidth)
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous).trim(from: 0, to: max(0, min(1, progress))).stroke(colour, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
        }
        .accessibilityLabel("Time left \(Int(progress * 100)) percent")
    }
}

/// A ring that draws, then a tick that strokes in. `progress` 0...1 drives both, so it can be frozen for review.
public struct CheckmarkDraw: View {
    let progress: Double, colour: Color
    public init(progress: Double, colour: Color = VUI.mint) { self.progress = progress; self.colour = colour }
    public var body: some View {
        let ring = min(1, progress / 0.55)
        let tick = max(0, min(1, (progress - 0.45) / 0.45))
        ZStack {
            Circle().fill(colour.opacity(0.14 * ring))
            Circle().trim(from: 0, to: ring).stroke(colour, style: StrokeStyle(lineWidth: 6, lineCap: .round)).rotationEffect(.degrees(-90))
            Tick().trim(from: 0, to: tick).stroke(colour, style: StrokeStyle(lineWidth: 9, lineCap: .round, lineJoin: .round)).padding(34)
        }
        .shadow(color: colour.opacity(0.45 * ring), radius: 24)
        .accessibilityHidden(true)
    }
    struct Tick: Shape {
        func path(in r: CGRect) -> Path {
            var p = Path()
            p.move(to: CGPoint(x: r.minX + r.width * 0.06, y: r.midY + r.height * 0.06))
            p.addLine(to: CGPoint(x: r.minX + r.width * 0.36, y: r.maxY - r.height * 0.06))
            p.addLine(to: CGPoint(x: r.maxX - r.width * 0.04, y: r.minY + r.height * 0.08))
            return p
        }
    }
}

/// Confetti as a pure function of `progress`: deterministic, so a frame can be rendered for review.
public struct ParticleBurst: View {
    let progress: Double, colours: [Color]
    public init(progress: Double, colours: [Color]) { self.progress = progress; self.colours = colours }
    public var body: some View {
        Canvas { ctx, size in
            let centre = CGPoint(x: size.width / 2, y: size.height / 2)
            for i in 0..<34 {
                let a = Double(i) * 2.399963 // golden angle: even spread, no clumps
                let speed = 60 + Double((i * 37) % 90)
                let t = progress
                // kept as small steps: one long expression times out the type checker on some Swift versions
                let dx: Double = cos(a) * speed * t * 1.2
                let dy: Double = sin(a) * speed * t * 1.2
                let drop: Double = 70 * t * t
                let x: CGFloat = centre.x + CGFloat(dx)
                let y: CGFloat = centre.y + CGFloat(dy + drop)
                let fade = max(0, 1 - max(0, t - 0.55) / 0.45)
                guard fade > 0 else { continue }
                var c = ctx
                c.translateBy(x: x, y: y); c.rotate(by: .radians(a + t * 9))
                let w: CGFloat = 5 + CGFloat(i % 4) * 2, h: CGFloat = 9 + CGFloat(i % 3) * 3
                c.fill(Path(roundedRect: CGRect(x: -w / 2, y: -h / 2, width: w, height: h), cornerRadius: 2), with: .color(colours[i % colours.count].opacity(fade)))
            }
        }
        .allowsHitTesting(false).accessibilityHidden(true)
    }
}

public struct Sparkline: View {
    let values: [Double], colour: Color
    public init(_ values: [Double], colour: Color) { self.values = values; self.colour = colour }
    public var body: some View {
        GeometryReader { g in
            let pts = points(g.size)
            ZStack {
                if pts.count > 1 {
                    Path { p in p.move(to: CGPoint(x: pts[0].x, y: g.size.height)); pts.forEach { p.addLine(to: $0) }; p.addLine(to: CGPoint(x: pts.last!.x, y: g.size.height)) }
                        .fill(LinearGradient(colors: [colour.opacity(0.35), colour.opacity(0)], startPoint: .top, endPoint: .bottom))
                    Path { p in p.move(to: pts[0]); pts.dropFirst().forEach { p.addLine(to: $0) } }
                        .stroke(colour, style: StrokeStyle(lineWidth: 2.5, lineCap: .round, lineJoin: .round))
                    Circle().fill(colour).frame(width: 8, height: 8).position(pts.last!)
                }
            }
        }
        .accessibilityHidden(true)
    }
    private func points(_ s: CGSize) -> [CGPoint] {
        guard values.count > 1, let lo = values.min(), let hi = values.max() else { return [] }
        let span = max(hi - lo, 1)
        return values.enumerated().map { i, v in CGPoint(x: s.width * CGFloat(i) / CGFloat(values.count - 1), y: s.height * (1 - 0.12 - 0.76 * CGFloat((v - lo) / span))) }
    }
}
