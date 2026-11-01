import SwiftUI
import VayoraKit
import VayoraUI

@main
struct VayoraMerchantApp: App {
    @StateObject private var store = MerchantStore()
    var body: some Scene {
        WindowGroup {
            TabView {
                CollectView().tabItem { Label("Collect", systemImage: "qrcode") }
                PaymentsView().tabItem { Label("Payments", systemImage: "list.bullet.rectangle") }
                if !store.cashierLocked {
                    AutoPayView().tabItem { Label("AutoPay", systemImage: "arrow.triangle.2.circlepath") }
                    MerchantSettings().tabItem { Label("Settings", systemImage: "gearshape.fill") }
                }
            }
            .environmentObject(store).tint(VUI.emerald).preferredColorScheme(.dark).dynamicTypeSize(...DynamicTypeSize.accessibility2)
        }
    }
}
