import SwiftUI

/// Shared look for both apps. Colours adapt to light and dark; nothing here is brand-critical text on a
/// low-contrast fill.
public enum Theme {
    public static let accent = Color(red: 0.36, green: 0.27, blue: 0.93)       // indigo
    public static let accentDeep = Color(red: 0.22, green: 0.15, blue: 0.68)
    public static let merchant = Color(red: 0.07, green: 0.62, blue: 0.46)     // emerald
    public static let merchantDeep = Color(red: 0.02, green: 0.34, blue: 0.25)
    public static let success = Color(red: 0.07, green: 0.62, blue: 0.40)
    public static let danger = Color(red: 0.84, green: 0.20, blue: 0.24)
    public static let warn = Color(red: 0.85, green: 0.55, blue: 0.05)
    #if canImport(UIKit)
    public static let card = Color(uiColor: .secondarySystemGroupedBackground)
    public static let canvas = Color(uiColor: .systemGroupedBackground)
    #else
    public static let card = Color(nsColor: .controlBackgroundColor)
    public static let canvas = Color(nsColor: .windowBackgroundColor)
    #endif

    public static var merchantGradient: LinearGradient {
        LinearGradient(colors: [merchant, merchantDeep], startPoint: .topLeading, endPoint: .bottomTrailing)
    }

    public static var heroGradient: LinearGradient {
        LinearGradient(colors: [accent, accentDeep], startPoint: .topLeading, endPoint: .bottomTrailing)
    }
}

/// A persistent, honest label: nothing in these apps moves real money.
public struct TestModeChip: View {
    public init() {}
    public var body: some View {
        Label("Test mode · no real money", systemImage: "flask.fill")
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 10).padding(.vertical, 5)
            .background(Theme.warn.opacity(0.16), in: Capsule())
            .foregroundStyle(Theme.warn)
            .accessibilityLabel("Test mode. No real money moves.")
    }
}

public struct PrimaryButtonStyle: ButtonStyle {
    let gradient: LinearGradient
    public init(gradient: LinearGradient = Theme.heroGradient) { self.gradient = gradient }
    public func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .frame(maxWidth: .infinity).padding(.vertical, 16)
            .background(gradient, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .foregroundStyle(.white)
            .opacity(configuration.isPressed ? 0.85 : 1)
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .animation(.spring(duration: 0.2), value: configuration.isPressed)
    }
}

public struct Card<Content: View>: View {
    let content: Content
    public init(@ViewBuilder _ content: () -> Content) { self.content = content() }
    public var body: some View {
        content.padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
    }
}
