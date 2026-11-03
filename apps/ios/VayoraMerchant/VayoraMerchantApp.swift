import SwiftUI
import VayoraKit
import VayoraUI

@main
struct VayoraMerchantApp: App {
    @StateObject private var store = MerchantStore()
    @Environment(\.scenePhase) private var phase
    var body: some Scene {
        WindowGroup {
            TabView {
                CollectView().tabItem { Label("Collect", systemImage: "qrcode") }
                PaymentsView().tabItem { Label("Payments", systemImage: "list.bullet.rectangle") }
                if !store.cashierLocked {
                    OwedView().tabItem { Label("Owed", systemImage: "indianrupeesign.circle") }.badge(store.attention.total)
                    AutoPayView().tabItem { Label("AutoPay", systemImage: "arrow.triangle.2.circlepath") }
                    MerchantSettings().tabItem { Label("Settings", systemImage: "gearshape.fill") }
                }
            }
            .task(id: phase) { if phase == .active { await store.refreshAttention() } }
            .onChange(of: store.cashierLocked) { _, _ in Task { await store.refreshAttention() } }
            .environmentObject(store).tint(VUI.emerald).preferredColorScheme(.dark).dynamicTypeSize(...DynamicTypeSize.accessibility2)
        }
    }
}
