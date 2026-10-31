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
    @EnvironmentObject var store: Store
    var done: () -> Void
    @State private var address = ""
    @State private var check: Check = .idle
    enum Check: Equatable { case idle, checking, ok, failed(String) }

    private var url: URL? {
        let t = address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let u = URL(string: t), let h = u.host, !h.isEmpty, u.scheme == "http" || u.scheme == "https" else { return nil }
        return u
    }

    private func test() {
        guard let u = url else { return }
        check = .checking
        Task {
            var r = URLRequest(url: u.appendingPathComponent("healthz")); r.timeoutInterval = 6
            do {
                let (_, resp) = try await URLSession.shared.data(for: r)
                let code = (resp as? HTTPURLResponse)?.statusCode ?? 0
                if code == 200 { store.serverURL = u.absoluteString; check = .ok } else { check = .failed("The server answered \(code).") }
            } catch {
                check = .failed("Couldn't reach it. Is the phone on the same Wi-Fi? (\((error as NSError).code))")
            }
        }
    }

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
            Card {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Test server", systemImage: "server.rack").font(.headline)
                    TextField("http://192.168.1.10:8080", text: $address)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                        .padding(10).background(Theme.canvas, in: RoundedRectangle(cornerRadius: 10))
                        .onChange(of: address) { _, _ in check = .idle }
                    HStack {
                        Button("Test connection", action: test).disabled(url == nil || check == .checking)
                        switch check {
                        case .idle: EmptyView()
                        case .checking: ProgressView()
                        case .ok: Label("Connected", systemImage: "checkmark.circle.fill").foregroundStyle(Theme.success)
                        case .failed: Label("Failed", systemImage: "xmark.circle.fill").foregroundStyle(Theme.danger)
                        }
                    }.font(.subheadline.weight(.semibold))
                    if case .failed(let why) = check { Text(why).font(.footnote).foregroundStyle(.secondary) }
                }
            }
            Spacer()
            Button("Get started", action: done).buttonStyle(PrimaryButtonStyle()).disabled(check != .ok)
                .opacity(check == .ok ? 1 : 0.5)
        }
        .padding(24)
    }
}
