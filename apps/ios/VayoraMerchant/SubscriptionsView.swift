import SwiftUI
import VayoraKit
import VayoraUI

/// Subscriptions: pause, resume or end them. In a test organisation, also move the test clock forward.
struct SubscriptionsView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var subs: [SubscriptionInfo] = []
    @State private var plans: [String: PlanInfo] = [:]
    @State private var clock: TestClock?
    @State private var message: String?
    @State private var notice: String?
    @State private var ending: SubscriptionInfo?
    @State private var loaded = false

    var isTestKey: Bool { store.apiKey.hasPrefix("zk_test") }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.35)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Subscriptions").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    if let message { Text(message).vfont(13).foregroundStyle(VUI.coral).accessibilityLabel("Error: \(message)") }
                    if let notice { Text(notice).vfont(13).foregroundStyle(VUI.mint) }
                    if isTestKey, let clock { clockCard(clock) }
                    if loaded, subs.isEmpty, message == nil { GlassCard { Text("No subscriptions yet.").vfont(14).foregroundStyle(VUI.textSecondary) } }
                    ForEach(subs) { s in row(s) }
                    Text("Creating subscriptions and changing plans is done on the web console.").vfont(12).foregroundStyle(VUI.textTertiary)
                }.padding(20)
            }
            .refreshable { await load() }
        }
        .environment(\.colorScheme, .dark)
        .task { await load() }
        .confirmationDialog("End this subscription?", isPresented: Binding(get: { ending != nil }, set: { if !$0 { ending = nil } }), titleVisibility: .visible, presenting: ending) { s in
            Button("End subscription", role: .destructive) { act(s, "ended") { try await $0.cancelSubscription(id: s.id) } }
            Button("Keep it", role: .cancel) {}
        } message: { _ in Text("Billing stops. This cannot be undone from the phone.") }
    }

    func row(_ s: SubscriptionInfo) -> some View {
        let plan = plans[s.planID]
        return GlassCard {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(plan?.name ?? "Plan").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                    Spacer()
                    Text(s.status.capitalized).vfont(12, .semibold).foregroundStyle(s.status == "active" ? VUI.mint : VUI.textTertiary)
                        .padding(.horizontal, 10).padding(.vertical, 5).background(.white.opacity(0.08), in: Capsule())
                }
                if let plan { Text("\(plan.priceLine)\(s.quantity > 1 ? " × \(s.quantity)" : "")").vfont(13).foregroundStyle(VUI.textSecondary) }
                Text([s.trialEndsOn.map { "Trial until \($0)" }, s.nextBillDate.map { "Next bill \($0)" }].compactMap { $0 }.joined(separator: " · ")).vfont(12).foregroundStyle(VUI.textTertiary)
                HStack(spacing: 16) {
                    if s.canPause { Button("Pause") { act(s, "paused") { try await $0.pauseSubscription(id: s.id) } } }
                    if s.canResume { Button("Resume") { act(s, "resumed") { try await $0.resumeSubscription(id: s.id) } } }
                    if s.canCancel { Button("End", role: .destructive) { ending = s } }
                }.vfont(14, .semibold).foregroundStyle(VUI.mint)
            }
        }
    }

    func clockCard(_ c: TestClock) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 8) {
                Label("Test clock", systemImage: "clock.arrow.2.circlepath").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                Text(c.frozen ? "Your test organisation thinks it is \(c.date) (the real date is \(c.realDate))." : "Following the real date, \(c.date).").vfont(13).foregroundStyle(VUI.textSecondary)
                HStack(spacing: 12) {
                    ForEach([1, 7, 30], id: \.self) { d in Button("+\(d) day\(d == 1 ? "" : "s")") { move(days: d) } }
                    if c.frozen { Button("Reset", role: .destructive) { move(days: nil) } }
                }.vfont(14, .semibold).foregroundStyle(VUI.mint)
                Text("Billing and reminders in this test organisation follow it. It only moves forward. Test environments only.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
        }
    }

    func load() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; loaded = true; return }
        do {
            async let s = c.subscriptions(); async let p = c.plans()
            let (subList, planList) = try await (s, p)
            subs = subList; plans = Dictionary(uniqueKeysWithValues: planList.map { ($0.id, $0) }); message = nil
        } catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
        if isTestKey { clock = try? await c.testClock() }
        loaded = true
    }

    func act(_ s: SubscriptionInfo, _ did: String, _ call: @escaping (VayoraClient) async throws -> Void) {
        guard let c = store.client else { return }
        Task {
            do { try await call(c); notice = "Subscription \(did)."; message = nil; Haptics.success(); await load() }
            catch { message = (error as? APIError)?.message ?? "Couldn't save that."; notice = nil; Haptics.error() }
        }
    }

    func move(days: Int?) {
        guard let c = store.client else { return }
        Task {
            do { if let days { clock = try await c.advanceTestClock(days: days) } else { clock = try await c.resetTestClock() }; notice = days == nil ? "Clock back on the real date." : "Moved forward \(days ?? 0) day\(days == 1 ? "" : "s")."; message = nil; Haptics.tap(); await load() }
            catch { message = (error as? APIError)?.message ?? "Couldn't move the clock."; notice = nil; Haptics.error() }
        }
    }
}
