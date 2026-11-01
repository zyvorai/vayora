import SwiftUI
import VayoraKit

/// A finished payment as a picture to share. Fixed size so it renders the same everywhere; says TEST on it.
public struct PaymentCardView: View {
    let m: PaymentCardModel
    public init(_ m: PaymentCardModel) { self.m = m }

    public var body: some View {
        ZStack {
            LinearGradient(colors: [VUI.indigoDeep, VUI.ink], startPoint: .topLeading, endPoint: .bottomTrailing)
            VStack(alignment: .leading, spacing: 14) {
                HStack {
                    Text("Vayora Pay").vfont(18, .bold)
                    Spacer()
                    Text("TEST MONEY").vfont(12, .bold).padding(.horizontal, 10).padding(.vertical, 5).background(.white.opacity(0.14), in: Capsule())
                }
                Spacer(minLength: 8)
                Image(systemName: m.paid ? "checkmark.circle.fill" : "xmark.circle.fill")
                    .font(.system(size: 54)).foregroundStyle(m.paid ? VUI.mint : VUI.coral)
                Text(m.amount).vfont(52, .bold).minimumScaleFactor(0.5).lineLimit(1)
                Text(m.headline).vfont(18, .semibold).foregroundStyle(m.paid ? VUI.mint : VUI.coral)
                VStack(alignment: .leading, spacing: 4) {
                    Text("to \(m.to)").vfont(20, .semibold).lineLimit(1)
                    if let note = m.note { Text(note).vfont(15).foregroundStyle(.white.opacity(0.75)).lineLimit(2) }
                    Text(m.when).vfont(14).foregroundStyle(.white.opacity(0.65))
                }
                Spacer(minLength: 8)
                Text("No real money moved.").vfont(12).foregroundStyle(.white.opacity(0.55))
            }
            .foregroundStyle(.white)
            .padding(28)
        }
        .frame(width: 360, height: 450)
        .clipShape(RoundedRectangle(cornerRadius: 28, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(m.headline), \(m.amount) to \(m.to), \(m.when). Test money.")
    }
}
