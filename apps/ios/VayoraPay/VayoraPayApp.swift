import SwiftUI
import VayoraKit

@main
struct VayoraPayApp: App {
    @StateObject private var store = Store()
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(store).tint(Theme.accent)
        }
    }
}

struct RootView: View {
    @EnvironmentObject var store: Store
    @AppStorage("onboarded") private var onboarded = false
    var body: some View {
        if onboarded {
            TabView {
                HomeView().tabItem { Label("Home", systemImage: "house.fill") }
                HistoryView().tabItem { Label("History", systemImage: "clock.fill") }
                SettingsView().tabItem { Label("Settings", systemImage: "gearshape.fill") }
            }
        } else {
            Onboarding { onboarded = true }
        }
    }
}

struct Onboarding: View {
    var done: () -> Void
    var body: some View {
        VStack(spacing: 24) {
            Spacer()
            Image(systemName: "qrcode.viewfinder").font(.system(size: 84)).foregroundStyle(Theme.heroGradient)
            Text("Vayora Pay").font(.largeTitle.bold())
            Text("A UPI-style payer app for testing Vayora Payments end to end.")
                .multilineTextAlignment(.center).foregroundStyle(.secondary)
            Card {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Test money only", systemImage: "flask.fill").font(.headline)
                    Text("This is not a bank-linked UPI app. Your ₹10,000 balance is a test balance on this phone, and payments go to Vayora's simulator.")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
            }
            Spacer()
            Button("Get started", action: done).buttonStyle(PrimaryButtonStyle())
        }
        .padding(24)
    }
}
