import SwiftUI
import VayoraKit

@main
struct VayoraMerchantApp: App {
    @StateObject private var store = MerchantStore()
    var body: some Scene {
        WindowGroup {
            TabView {
                CollectView().tabItem { Label("Collect", systemImage: "qrcode") }
                PaymentsView().tabItem { Label("Payments", systemImage: "list.bullet.rectangle") }
                MerchantSettings().tabItem { Label("Settings", systemImage: "gearshape.fill") }
            }
            .environmentObject(store).tint(Theme.merchant)
        }
    }
}
