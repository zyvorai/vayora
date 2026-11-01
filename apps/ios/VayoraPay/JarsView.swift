import SwiftUI
import VayoraKit
import VayoraUI

/// Set test money aside for a goal. It moves between this wallet and a jar and is never created or lost; no bank, no interest.
struct JarsView: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    @State private var name = ""
    @State private var target = ""
    @State private var amounts: [UUID: String] = [:]
    @State private var message: String?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.4)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Jars").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    Text("Wallet \(store.balance.display). Money in jars is set aside and cannot be spent until you move it back.").vfont(14).foregroundStyle(VUI.textSecondary)
                    ForEach(store.jars.jars) { j in jarCard(j) }
                    if store.jars.jars.count < JarBook.maxJars { addCard }
                    if let message { Text(message).vfont(13).foregroundStyle(VUI.coral) }
                    Text("Test money on this phone. Not a bank account; no interest.").vfont(12).foregroundStyle(VUI.textTertiary)
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
    }

    func jarCard(_ j: Jar) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                HStack { Text(j.name).vfont(17, .semibold).foregroundStyle(VUI.textPrimary); Spacer()
                    Text("\(Money(minor: j.savedMinor).display) of \(Money(minor: j.targetMinor).display)").vfont(13).foregroundStyle(j.reached ? VUI.mint : VUI.textSecondary) }
                ProgressView(value: j.fraction).tint(j.reached ? VUI.mint : VUI.violet)
                    .accessibilityLabel("\(Int(j.fraction * 100)) percent of the goal")
                if j.reached { Label("Goal reached", systemImage: "checkmark.seal.fill").vfont(13, .semibold).foregroundStyle(VUI.mint) }
                TextField("Amount in ₹", text: Binding(get: { amounts[j.id] ?? "" }, set: { amounts[j.id] = $0 })).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary)
                    .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                HStack(spacing: 16) {
                    Button("Add to jar") { move(j.id, deposit: true) }
                    Button("Take out") { move(j.id, deposit: false) }
                    Spacer()
                    Button("Delete", role: .destructive) { store.removeJar(j.id); Haptics.success() }
                }.vfont(14, .semibold).foregroundStyle(VUI.violet)
            }
        }
    }

    var addCard: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                Label("New jar", systemImage: "plus.circle.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                TextField("Name (Trip, Rent…)", text: $name).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                TextField("Goal in ₹", text: $target).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                Button("Create jar") {
                    guard let t = Money(decimal: target) else { message = "Enter a goal amount."; Haptics.error(); return }
                    do { var b = store.jars; try b.add(name: name, targetMinor: t.minor); store.jars = b; name = ""; target = ""; message = nil; Haptics.success() }
                    catch { message = (error as? JarBook.Failure) == .badName ? "Give it a name up to 30 characters." : "That goal isn't valid."; Haptics.error() }
                }.vfont(15, .semibold).foregroundStyle(VUI.violet)
            }
        }
    }

    func move(_ id: UUID, deposit: Bool) {
        guard let m = Money(decimal: amounts[id] ?? "") else { message = "Enter an amount."; Haptics.error(); return }
        let f = deposit ? store.moveToJar(id, m.minor) : store.moveFromJar(id, m.minor)
        switch f {
        case nil: amounts[id] = ""; message = nil; Haptics.success()
        case .walletTooLow: message = "Your wallet doesn't have that much."; Haptics.error()
        case .jarTooLow: message = "The jar doesn't hold that much."; Haptics.error()
        default: message = "That didn't work."; Haptics.error()
        }
    }
}
