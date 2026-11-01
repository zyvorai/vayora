import SwiftUI
import VayoraKit
import VayoraUI

/// A week of this phone's own history in plain sentences. Page back to earlier weeks; share as text.
struct RecapView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var offset = 0 // weeks back from this one

    var day: Date { Calendar.current.date(byAdding: .day, value: -7 * offset, to: Date()) ?? Date() }
    var recap: WeeklyRecap { WeeklyRecap.make(store.historyRows, weekContaining: day) }

    var body: some View {
        let r = recap
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.5)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Your week").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 44, height: 44) }
                            .accessibilityLabel("Close")
                    }
                    HStack {
                        Button { offset += 1 } label: { Image(systemName: "chevron.left").frame(width: 44, height: 44) }.accessibilityLabel("Earlier week")
                        Spacer()
                        Text(title(r)).vfont(15, .semibold).foregroundStyle(VUI.textPrimary)
                        Spacer()
                        Button { offset = max(0, offset - 1) } label: { Image(systemName: "chevron.right").frame(width: 44, height: 44) }
                            .disabled(offset == 0).accessibilityLabel("Later week")
                    }.foregroundStyle(VUI.violet)
                    if !r.isEmpty { AmountText(Money(minor: r.totalMinor), size: 48) }
                    GlassCard {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(Array(r.lines.enumerated()), id: \.offset) { _, l in Text(l).vfont(16).foregroundStyle(VUI.textPrimary).fixedSize(horizontal: false, vertical: true) }
                        }
                    }
                    ShareLink(item: ("Vayora Pay (TEST) · " + title(r) + "\n" + r.lines.joined(separator: "\n"))) {
                        Label("Share this week", systemImage: "square.and.arrow.up").vfont(16, .semibold).foregroundStyle(VUI.violet)
                    }
                }.padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
    }

    func title(_ r: WeeklyRecap) -> String {
        let f = DateFormatter(); f.dateFormat = "d MMM"
        let last = Calendar.current.date(byAdding: .day, value: -1, to: r.weekEndExclusive) ?? r.weekEndExclusive
        return "\(f.string(from: r.weekStart)) – \(f.string(from: last))"
    }
}
