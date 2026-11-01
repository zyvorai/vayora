import SwiftUI
import VayoraKit
import VayoraUI

/// Close the day: log cash movements, count the till, see expected against counted. A second person signs it off on the console.
struct EndOfDayView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var branches: [BranchInfo] = []
    @State private var branch: BranchInfo?
    @State private var kind = "sale"
    @State private var cashText = ""
    @State private var countedText = ""
    @State private var message: String?
    @State private var closing: ClosingInfo?
    @State private var working = false
    @State private var newBranch = ""

    /// Today in India, whatever the phone's own time zone says: closings are per IST day.
    var today: String {
        var c = Calendar(identifier: .gregorian); c.timeZone = TimeZone(identifier: "Asia/Kolkata")!
        let d = c.dateComponents([.year, .month, .day], from: Date())
        return String(format: "%04d-%02d-%02d", d.year ?? 0, d.month ?? 0, d.day ?? 0)
    }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.6)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    HStack {
                        Text("End of day").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    if let c = closing { result(c) } else { form }
                    if let message { Text(message).vfont(14).foregroundStyle(VUI.textSecondary) }
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .task { await loadBranches() }
    }

    var form: some View {
        VStack(alignment: .leading, spacing: 16) {
            GlassCard {
                VStack(alignment: .leading, spacing: 10) {
                    Label("Branch", systemImage: "building.2").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                    if branches.isEmpty {
                        Text("No branches yet. Name your shop or stall to start.").vfont(14).foregroundStyle(VUI.textSecondary)
                        field("Branch name", $newBranch)
                        Button("Add branch", action: addBranch).vfont(15, .semibold).foregroundStyle(VUI.mint).disabled(newBranch.trimmingCharacters(in: .whitespaces).isEmpty || working)
                    }
                    ForEach(branches) { b in
                        Button { branch = b } label: {
                            HStack { Text(b.name).foregroundStyle(VUI.textPrimary); Spacer(); if branch == b { Image(systemName: "checkmark.circle.fill").foregroundStyle(VUI.mint) } }
                                .vfont(16).padding(.vertical, 6)
                        }
                    }
                    Text("Day: \(today)").vfont(13).foregroundStyle(VUI.textTertiary)
                }
            }
            GlassCard {
                VStack(alignment: .leading, spacing: 10) {
                    Label("Cash in the till", systemImage: "indianrupeesign.circle").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                    Picker("Kind", selection: $kind) { Text("Cash sale").tag("sale"); Text("Handed back").tag("refund"); Text("Float in").tag("float") }.pickerStyle(.segmented)
                    field("Amount in ₹", $cashText)
                    Button(action: addCash) { Text("Record").vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 12)
                        .background(LinearGradient(colors: [VUI.mint, VUI.emerald], startPoint: .top, endPoint: .bottom), in: RoundedRectangle(cornerRadius: 16, style: .continuous)) }
                        .buttonStyle(PressScale()).opacity(Money(decimal: cashText) != nil && branch != nil && !working ? 1 : 0.4).disabled(Money(decimal: cashText) == nil || branch == nil || working)
                    Text("Entries are never edited. To fix one, record the opposite.").vfont(12).foregroundStyle(VUI.textTertiary)
                }
            }
            GlassCard {
                VStack(alignment: .leading, spacing: 10) {
                    Label("Count and close", systemImage: "checkmark.seal").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                    field("Cash counted in ₹", $countedText)
                    Button(action: close) { Text(working ? "Closing…" : "Close the day").vfont(16, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 14)
                        .background(Brand.merchant.gradient, in: RoundedRectangle(cornerRadius: 18, style: .continuous)) }
                        .buttonStyle(PressScale()).opacity(canClose ? 1 : 0.4).disabled(!canClose)
                    Text("A different person signs it off on the console. Test mode.").vfont(12).foregroundStyle(VUI.textTertiary)
                }
            }
        }
    }

    var canClose: Bool { (Money(decimal: countedText) != nil || countedText == "0") && branch != nil && !working }

    func field(_ title: String, _ text: Binding<String>) -> some View {
        TextField(title, text: text).keyboardType(.decimalPad).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    func result(_ c: ClosingInfo) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            GlassCard {
                VStack(alignment: .leading, spacing: 10) {
                    HStack { Text("Day closed").vfont(18, .semibold).foregroundStyle(VUI.textPrimary); Spacer(); StatusPill(c.verdict, c.differenceMinor == 0 ? .success : .warn) }
                    row("Expected cash", Money(minor: c.expectedCashMinor).display)
                    row("Counted cash", Money(minor: c.countedCashMinor).display)
                    row("Difference", (c.differenceMinor > 0 ? "+" : c.differenceMinor < 0 ? "-" : "") + Money(minor: abs(c.differenceMinor)).display)
                    ForEach(c.digital, id: \.method) { d in row("\(d.method.uppercased()) (\(d.payments))", Money(minor: d.netMinor).display) }
                }
            }
            Text("Waiting for a second person to sign off on the console.").vfont(13).foregroundStyle(VUI.textSecondary)
            Button { dismiss() } label: { Text("Done").vfont(17, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 15).background(Brand.merchant.gradient, in: RoundedRectangle(cornerRadius: 20, style: .continuous)) }.buttonStyle(PressScale())
        }
    }
    func row(_ k: String, _ v: String) -> some View { HStack { Text(k).foregroundStyle(VUI.textSecondary); Spacer(); Text(v).foregroundStyle(VUI.textPrimary) }.vfont(15) }

    func loadBranches() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; return }
        do { branches = try await c.branches(); if branches.count == 1 { branch = branches.first } }
        catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
    }
    func addBranch() {
        guard let c = store.client else { return }
        working = true
        Task {
            defer { working = false }
            do { let b = try await c.createBranch(name: newBranch); branches = [b]; branch = b; newBranch = ""; message = nil; Haptics.success() }
            catch { message = (error as? APIError)?.message ?? "Couldn't reach the server."; Haptics.error() }
        }
    }
    func addCash() {
        guard let c = store.client, let b = branch, let m = Money(decimal: cashText) else { return }
        working = true
        Task {
            defer { working = false }
            do { try await c.addCash(branchID: b.id, kind: kind, amount: m); message = "Recorded \(m.display)."; cashText = ""; Haptics.success() }
            catch { message = (error as? APIError)?.message ?? "Couldn't reach the server."; Haptics.error() }
        }
    }
    func close() {
        guard let c = store.client, let b = branch else { return }
        let counted = Money(decimal: countedText) ?? Money(minor: 0)
        working = true
        Task {
            defer { working = false }
            do { closing = try await c.closeDay(branchID: b.id, day: today, counted: counted); message = nil; Haptics.success() }
            catch { message = (error as? APIError)?.message ?? "Couldn't reach the server."; Haptics.error() }
        }
    }
}
