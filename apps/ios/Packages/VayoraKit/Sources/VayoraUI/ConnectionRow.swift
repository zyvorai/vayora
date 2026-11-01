import SwiftUI
import VayoraKit

/// "Test connection": one press, one plain answer. The app supplies the client (nil when no server is set).
public struct ConnectionRow: View {
    let client: () -> VayoraClient?
    let needsKey: Bool
    @State private var running = false
    @State private var verdict: ConnectionVerdict?

    public init(needsKey: Bool = false, client: @escaping () -> VayoraClient?) { self.needsKey = needsKey; self.client = client }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                Task {
                    running = true; verdict = nil
                    if let c = client() { verdict = await c.checkConnection(needsKey: needsKey) } else { verdict = .badAddress }
                    running = false
                }
            } label: {
                HStack(spacing: 8) {
                    if running { ProgressView().controlSize(.small) } else { Image(systemName: "antenna.radiowaves.left.and.right") }
                    Text(running ? "Checking…" : "Test connection")
                }
            }
            .vfont(15, .semibold).foregroundStyle(VUI.violet).disabled(running)
            if let verdict {
                VStack(alignment: .leading, spacing: 4) {
                    Label(verdict.title, systemImage: verdict.isOK ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                        .vfont(14, .semibold).foregroundStyle(verdict.isOK ? VUI.mint : VUI.textPrimary)
                    Text(verdict.advice).vfont(13).foregroundStyle(VUI.textSecondary)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}
