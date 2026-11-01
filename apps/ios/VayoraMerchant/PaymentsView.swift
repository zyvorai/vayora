import SwiftUI
import VayoraKit
import VayoraUI

struct PaymentsView: View {
    @EnvironmentObject var store: MerchantStore
    @State private var items: [PaymentIntent] = []
    @State private var refunds: [String: RefundState] = [:]
    @State private var error: String?
    @State private var open: PaymentIntent?

    func state(_ i: PaymentIntent) -> PaymentRowModel.State {
        if let r = refunds[i.id], r.isFullyRefunded { return .refunded }
        if let r = refunds[i.id], r.isPartlyRefunded { return .partRefunded }
        switch i.status { case "succeeded": return .succeeded; case "processing", "requires_payment_method", "requires_action": return .processing; default: return .failed }
    }

    var rows: [PaymentRowModel] {
        items.map { PaymentRowModel(id: $0.id, amount: $0.money, when: when($0), note: "", state: state($0)) }
    }
    func when(_ i: PaymentIntent) -> String {
        guard let s = i.createdAt, let d = ISO8601DateFormatter().date(from: s) ?? ISO8601DateFormatter.withFraction.date(from: s) else { return String(i.id.suffix(6)) }
        return d.formatted(.relative(presentation: .named))
    }

    /// Money kept after refunds: paid payments minus what has been given back.
    var paid: [PaymentIntent] { items.filter { $0.status == "succeeded" } }
    var collected: Money {
        let back = paid.reduce(Int64(0)) { $0 + (refunds[$1.id]?.refundedMinor ?? 0) }
        return Money(minor: paid.reduce(Int64(0)) { $0 + $1.amountMinor } - back)
    }
    /// Running total of what was kept, oldest payment first, for the sparkline.
    var trend: [Double] {
        var run = 0.0
        return paid.reversed().map { i in run += Double(i.amountMinor - (refunds[i.id]?.refundedMinor ?? 0)) / 100; return run }
    }

    /// Today's money by hour, over the hours that matter. Net of nothing: refunds show in the total, not here.
    var hourly: HourlyBars.Model? {
        let iso = ISO8601DateFormatter(), frac = ISO8601DateFormatter.withFraction
        let pairs: [(Date, Int64)] = paid.compactMap { i in
            guard let s = i.createdAt, let d = iso.date(from: s) ?? frac.date(from: s) else { return nil }
            return (d, i.amountMinor)
        }
        let totals = HourlyTotals.make(pairs, day: Date())
        guard totals.contains(where: { $0 > 0 }) else { return nil }
        let w = HourlyTotals.window(totals)
        return HourlyBars.Model(hours: Array(w), minor: w.map { totals[$0] })
    }

    var body: some View {
        NavigationStack {
            MerchantTodayScreen(collected: collected, count: paid.count, average: Money(minor: paid.isEmpty ? 0 : collected.minor / Int64(paid.count)),
                                trend: trend.count > 1 ? trend : [0, 0], rows: rows, error: error, hourly: hourly, onOpen: { r in open = items.first { $0.id == r.id } })
                .refreshable { await load() }
                .task { await load() }
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(item: $open) { PaymentDetail(intent: $0) }
        }
    }

    func load() async {
        guard let c = store.client else { error = "Add your API key in Settings."; return }
        do {
            items = try await c.listIntents(); error = nil
            // Unfinished payments only turn paid when the server is asked, so ask for the most recent ones.
            let unfinished = items.filter { $0.status == "processing" }.prefix(10).map(\.id)
            if !unfinished.isEmpty {
                await withTaskGroup(of: Void.self) { g in
                    for id in unfinished { g.addTask { _ = try? await c.settle(id: id) } }
                }
                items = try await c.listIntents()
            }
            // Refund position of the most recent paid payments, so the list can say "refunded".
            let ids = items.filter { $0.status == "succeeded" }.prefix(20).map(\.id)
            refunds = await withTaskGroup(of: (String, RefundState?).self) { g -> [String: RefundState] in
                for id in ids { g.addTask { (id, try? await c.refundState(id: id)) } }
                var out: [String: RefundState] = [:]
                for await (id, s) in g { if let s { out[id] = s } }
                return out
            }
        } catch { self.error = error.localizedDescription }
    }
}

extension ISO8601DateFormatter {
    static let withFraction: ISO8601DateFormatter = { let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return f }()
}

struct PaymentDetail: View {
    @EnvironmentObject var store: MerchantStore
    let intent: PaymentIntent
    @State private var state: RefundState?
    @State private var message: String?
    @State private var refunding = false
    @State private var confirm = false

    var created: Date? {
        guard let s = intent.createdAt else { return nil }
        return ISO8601DateFormatter().date(from: s) ?? ISO8601DateFormatter.withFraction.date(from: s)
    }

    var pill: (String, StatusPill.Kind) {
        if let s = state, s.isFullyRefunded { return ("Refunded", .warn) }
        if let s = state, s.isPartlyRefunded { return ("Part refunded", .warn) }
        return intent.status == "succeeded" ? ("Paid", .success) : (intent.status, .neutral)
    }

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.4)
            ScrollView {
                VStack(spacing: 18) {
                    AmountText(intent.money, size: 60).padding(.top, 24)
                    StatusPill(pill.0, pill.1)
                    GlassCard {
                        VStack(alignment: .leading, spacing: 12) {
                            row("Payment", intent.id)
                            if let s = state, s.refundedMinor > 0 { row("Refunded", Money(minor: s.refundedMinor, currency: intent.currency).display) }
                        }
                    }
                    if intent.status == "succeeded" {
                        ShareLink(item: ReceiptText.make(merchant: "", amount: intent.money, status: intent.status, id: intent.id, when: created, refundedMinor: state?.refundedMinor ?? 0)) {
                            Label("Share receipt", systemImage: "square.and.arrow.up").vfont(17, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 16)
                                .background(Brand.merchant.gradient, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                        }.buttonStyle(PressScale())
                    }
                    if intent.status == "succeeded", let s = state {
                        if s.remainingMinor > 0 {
                            Button(role: .destructive) { confirm = true } label: {
                                Text(s.isPartlyRefunded ? "Refund the remaining \(Money(minor: s.remainingMinor, currency: intent.currency).display)" : "Refund in full")
                                    .vfont(18, .semibold).foregroundStyle(VUI.coral).frame(maxWidth: .infinity).padding(.vertical, 17)
                                    .background(VUI.coral.opacity(0.14), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                                    .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(VUI.coral.opacity(0.35)))
                            }.buttonStyle(PressScale()).disabled(refunding)
                            .confirmationDialog("Refund this payment?", isPresented: $confirm, titleVisibility: .visible) {
                                Button("Refund \(Money(minor: s.remainingMinor, currency: intent.currency).display)", role: .destructive) { refund(s.remainingMinor) }
                            } message: { Text("Test mode: recorded against the simulator.") }
                        } else {
                            Label("Fully refunded", systemImage: "arrow.uturn.backward.circle.fill").vfont(17, .semibold).foregroundStyle(VUI.amber)
                        }
                    }
                    if let message { Text(message).vfont(14).foregroundStyle(VUI.textSecondary) }
                }
                .padding(20)
            }
        }
        .environment(\.colorScheme, .dark)
        .navigationTitle("Payment").navigationBarTitleDisplayMode(.inline)
        .task { await reload() }
    }

    func row(_ k: String, _ v: String) -> some View {
        HStack { Text(k).foregroundStyle(VUI.textSecondary); Spacer(); Text(v).foregroundStyle(VUI.textPrimary).lineLimit(1).truncationMode(.middle) }
            .vfont(15)
    }

    func reload() async {
        guard let c = store.client else { return }
        state = try? await c.refundState(id: intent.id)
    }

    func refund(_ remaining: Int64) {
        guard let c = store.client else { return }
        refunding = true
        Task {
            defer { refunding = false }
            do {
                let r = try await c.refund(intentId: intent.id, amount: Money(minor: remaining, currency: intent.currency), reason: "requested_by_merchant")
                message = "Refund \(r.status)"; Haptics.success()
            } catch { message = error.localizedDescription; Haptics.error() }
            await reload()
        }
    }
}

struct MerchantSettings: View {
    @EnvironmentObject var store: MerchantStore
    @State private var scanning = false
    @State private var setupMessage: String?
    @State private var closingDay = false
    @AppStorage("soundbox") private var soundbox = "off"

    var body: some View {
        NavigationStack {
            ZStack {
                AmbientBackground(brand: .merchant, phase: 0.5)
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Text("Settings").vfont(28, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("Counter speaker", systemImage: "speaker.wave.2.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Picker("Speak", selection: $soundbox) { Text("Off").tag("off"); Text("English").tag("en"); Text("हिन्दी").tag("hi") }.pickerStyle(.segmented)
                                Button("Hear it") { Soundbox.shared.speak(SoundboxPhrase.received(Money(minor: 250_00), language: soundbox == "hi" ? .hindi : .english), soundbox == "hi" ? .hindi : .english) }
                                    .vfont(15, .semibold).foregroundStyle(VUI.mint).disabled(soundbox == "off")
                                Text("Says the amount out loud when a payment is received. The phone's own voice; it plays even with the silent switch on.").vfont(12).foregroundStyle(VUI.textTertiary)
                                Button { closingDay = true } label: { Label("End of day", systemImage: "moon.stars.fill") }.vfont(15, .semibold).foregroundStyle(VUI.mint)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("Quick setup", systemImage: "qrcode.viewfinder").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Button { scanning = true } label: {
                                    Text("Scan setup QR").vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 14)
                                        .background(LinearGradient(colors: [VUI.mint, VUI.emerald], startPoint: .top, endPoint: .bottom), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                                }.buttonStyle(PressScale())
                                if let setupMessage { Text(setupMessage).vfont(13).foregroundStyle(VUI.textSecondary) }
                                Text("Fills in the server and a test key from a QR shown on your computer. Live keys are refused.").vfont(13).foregroundStyle(VUI.textTertiary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("Server", systemImage: "server.rack").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                TextField("https://api.example.com", text: $store.serverURL).textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                                    .foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 10) {
                                Label("API key", systemImage: "key.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                SecureField("zk_test_…", text: $store.apiKey).textInputAutocapitalization(.never).autocorrectionDisabled()
                                    .foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                                Text("Stored in the iPhone Keychain, never in app settings.").vfont(13).foregroundStyle(VUI.textSecondary)
                            }
                        }
                        GlassCard {
                            VStack(alignment: .leading, spacing: 8) {
                                Label("About", systemImage: "info.circle.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                                Text("Vayora Merchant creates test payments and shows their QR. With the simulator or Cashfree sandbox no real money moves.")
                                    .vfont(14).foregroundStyle(VUI.textSecondary)
                            }
                        }
                    }
                    .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 32)
                }
            }
            .environment(\.colorScheme, .dark)
            .toolbar(.hidden, for: .navigationBar)
            .sheet(isPresented: $closingDay) { EndOfDayView().environmentObject(store) }
            .fullScreenCover(isPresented: $scanning) {
                ScannerView(brand: .merchant, hint: "Scan the setup QR") { text in
                    scanning = false
                    do {
                        let p = try SetupPayload.parse(text)
                        store.serverURL = p.server.absoluteString
                        store.apiKey = p.apiKey
                        setupMessage = "Set up for \(p.server.host ?? "server"). Open Payments."
                        Haptics.success()
                    } catch SetupPayload.Failure.liveKeyRefused {
                        setupMessage = "That QR holds a live key. These apps are test-only, so it was refused."
                    } catch {
                        setupMessage = "That is not a Vayora setup QR."
                    }
                } onCancel: { scanning = false }
            }
        }
    }
}
