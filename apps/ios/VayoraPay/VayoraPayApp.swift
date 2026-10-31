import SwiftUI
import VayoraKit
import VayoraUI

@main
struct VayoraPayApp: App {
    @StateObject private var store = Store()
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(store).tint(VUI.violet).preferredColorScheme(.dark)
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
    @State private var scanning = false
    @State private var scanNote: String?
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
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.3)
            ScrollView {
                VStack(spacing: 22) {
                    Spacer(minLength: 30)
                    Image(systemName: "checkmark").font(.system(size: 54, weight: .heavy)).foregroundStyle(.white)
                        .frame(width: 112, height: 112).background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 28, style: .continuous).strokeBorder(.white.opacity(0.35)))
                        .shadow(color: VUI.indigo.opacity(0.6), radius: 28, y: 10)
                    Text("Vayora Pay").font(VUI.display(36)).foregroundStyle(VUI.textPrimary)
                    Text("Pay like a gesture. A UPI-style payer for testing Vayora Payments end to end.")
                        .font(.system(size: 16, design: .rounded)).multilineTextAlignment(.center).foregroundStyle(VUI.textSecondary).padding(.horizontal, 12)
                    GlassCard {
                        VStack(alignment: .leading, spacing: 8) {
                            Label("Test money only", systemImage: "flask.fill").font(VUI.display(17, .semibold)).foregroundStyle(VUI.amber)
                            Text("Not a bank-linked UPI app. Your ₹10,000 balance is a test balance on this phone, and payments go to Vayora's simulator.")
                                .font(.system(size: 14, design: .rounded)).foregroundStyle(VUI.textSecondary)
                        }
                    }
                    GlassCard {
                        VStack(alignment: .leading, spacing: 10) {
                            Label("Test server", systemImage: "server.rack").font(VUI.display(17, .semibold)).foregroundStyle(VUI.textPrimary)
                            TextField("http://192.168.1.10:8080", text: $address)
                                .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL).foregroundStyle(VUI.textPrimary)
                                .padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                .onChange(of: address) { _, _ in check = .idle }
                            Button { scanning = true } label: { Label("Scan setup QR", systemImage: "qrcode.viewfinder") }
                                .font(.system(size: 15, weight: .semibold, design: .rounded)).foregroundStyle(VUI.violet)
                            if let scanNote { Text(scanNote).font(.system(size: 13, design: .rounded)).foregroundStyle(VUI.textSecondary) }
                            HStack {
                                Button("Test connection", action: test).disabled(url == nil || check == .checking).foregroundStyle(VUI.violet)
                                switch check {
                                case .idle: EmptyView()
                                case .checking: ProgressView().tint(.white)
                                case .ok: Label("Connected", systemImage: "checkmark.circle.fill").foregroundStyle(VUI.mint)
                                case .failed: Label("Failed", systemImage: "xmark.circle.fill").foregroundStyle(VUI.coral)
                                }
                            }.font(.system(size: 15, weight: .semibold, design: .rounded))
                            if case .failed(let why) = check { Text(why).font(.system(size: 13, design: .rounded)).foregroundStyle(VUI.textSecondary) }
                        }
                    }
                    Button(action: done) {
                        Text("Get started").font(VUI.display(18, .semibold)).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17)
                            .background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                    }.buttonStyle(PressScale()).disabled(check != .ok).opacity(check == .ok ? 1 : 0.4)
                }
                .padding(24)
            }
        }
        .environment(\.colorScheme, .dark)
        .onAppear {
            // a server remembered from before (it survives a reinstall): fill it in and check it
            if address.isEmpty, URL(string: store.serverURL)?.host.map({ $0 != "127.0.0.1" }) == true { address = store.serverURL; test() }
        }
        .fullScreenCover(isPresented: $scanning) {
            ScannerView(brand: .pay, hint: "Scan the setup QR") { text in
                scanning = false
                if let p = try? SetupPayload.parse(text) { address = p.server.absoluteString; scanNote = nil; test() }
                else { scanNote = "That is not a Vayora setup QR." }
            } onCancel: { scanning = false }
        }
    }
}
