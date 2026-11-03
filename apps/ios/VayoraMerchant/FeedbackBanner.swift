import SwiftUI
import VayoraKit
import VayoraUI

/// What just happened (or went wrong), pinned to the top of a screen so it is seen however far the list below is scrolled.
/// Tap it to dismiss. An error wins over a notice. Use with `.safeAreaInset(edge: .top, spacing: 0) { FeedbackBanner(...) }`.
struct FeedbackBanner: View {
    @Binding var message: String?
    @Binding var notice: String?

    var body: some View {
        if let text = message {
            bar(text, VUI.coral, spoken: "Error: \(text)") { message = nil }
        } else if let text = notice {
            bar(text, VUI.mint, spoken: text) { notice = nil }
        }
    }

    func bar(_ text: String, _ fill: Color, spoken: String, dismiss: @escaping () -> Void) -> some View {
        Text(text).vfont(14, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity, alignment: .leading).padding(12)
            .background(fill, in: RoundedRectangle(cornerRadius: 14, style: .continuous)).padding(.horizontal, 16).padding(.top, 4)
            .accessibilityLabel(spoken).accessibilityHint("Double tap to dismiss").onTapGesture(perform: dismiss)
    }
}
