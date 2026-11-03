import SwiftUI
import VayoraKit
import VayoraUI

/// Payment links: make one for an amount, share its address or QR, email it, or cancel it.
struct PaymentLinksView: View {
    @EnvironmentObject var store: MerchantStore
    @Environment(\.dismiss) var dismiss
    @State private var links: [PaymentLinkInfo] = []
    @State private var loaded = false
    @State private var message: String?
    @State private var notice: String?
    @State private var entry = AmountEntry()
    @State private var note = ""
    @State private var oneUse = false
    @State private var busy = false
    @State private var shown: PaymentLinkInfo?
    @State private var customers: [CustomerInfo] = []
    @State private var customerID = ""

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.2)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack {
                        Text("Payment links").vfont(26, .bold).foregroundStyle(VUI.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer()
                        Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close")
                    }
                    newLink
                    if loaded, links.isEmpty { GlassCard { Text("No links yet. Make one above and share it with a customer.").vfont(14).foregroundStyle(VUI.textSecondary) } }
                    ForEach(links) { l in row(l) }
                    if store.demoMode { Text("Test money. The built-in demo cannot email a link.").vfont(12).foregroundStyle(VUI.textTertiary) }
                    else if URL(string: store.checkoutURL)?.scheme != "https" { Text("Sharing needs your public checkout address (https) in Settings. Emailing a link does not.").vfont(12).foregroundStyle(VUI.textTertiary) }
                }.padding(20)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) { FeedbackBanner(message: $message, notice: $notice) }
        .environment(\.colorScheme, .dark)
        .task { await load() }
        .sheet(item: $shown) { l in LinkShareSheet(link: l, url: l.shareURL(checkoutBase: store.checkoutURL)) }
    }

    var newLink: some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 10) {
                Label("New link", systemImage: "link.badge.plus").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                TextField("Amount in rupees", text: Binding(get: { entry.text }, set: { setAmount($0) })).keyboardType(.decimalPad)
                    .foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .accessibilityLabel("Amount in rupees")
                TextField("What it is for (optional)", text: $note).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                if !customers.isEmpty {
                    Picker("For", selection: $customerID) {
                        Text("No one in particular").tag("")
                        ForEach(customers) { Text($0.name).tag($0.id) }
                    }.pickerStyle(.menu).tint(VUI.mint).foregroundStyle(VUI.textPrimary)
                        .accessibilityHint("Choose a customer to be able to email them the link")
                }
                Toggle("Stop after one payment", isOn: $oneUse).tint(VUI.mint).foregroundStyle(VUI.textPrimary)
                Button(action: create) {
                    Text(busy ? "Creating…" : "Create link").vfont(16, .semibold).foregroundStyle(VUI.ink).frame(maxWidth: .infinity).padding(.vertical, 14).background(VUI.mint, in: Capsule())
                }.buttonStyle(PressScale()).disabled(entry.money == nil || busy || store.client == nil).opacity(entry.money == nil || busy ? 0.5 : 1)
            }
        }
    }

    func row(_ l: PaymentLinkInfo) -> some View {
        GlassCard {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(l.amountMinor.map { Money(minor: $0).display } ?? (l.invoiceID != nil ? "Invoice balance" : "Any amount")).vfont(18, .semibold).foregroundStyle(VUI.textPrimary)
                        if let d = l.description, !d.isEmpty { Text(d).vfont(13).foregroundStyle(VUI.textSecondary) }
                    }
                    Spacer()
                    Text(l.status.capitalized).vfont(12, .semibold).foregroundStyle(l.isActive ? VUI.mint : VUI.textTertiary)
                        .padding(.horizontal, 10).padding(.vertical, 5).background(.white.opacity(0.08), in: Capsule())
                }
                Text("Used \(l.uses)\(l.maxUses.map { " of \($0)" } ?? "") time\(l.uses == 1 && l.maxUses == nil ? "" : "s")").vfont(12).foregroundStyle(VUI.textTertiary)
                if l.isActive {
                    HStack(spacing: 14) {
                        Button("Share") { shown = l }.disabled(l.shareURL(checkoutBase: store.checkoutURL) == nil)
                            .accessibilityHint(l.shareURL(checkoutBase: store.checkoutURL) == nil ? "Set your checkout address in Settings first" : "Shows the link and its QR")
                        if l.canEmail { Button("Email") { email(l) } }
                        Button("Cancel link", role: .destructive) { cancel(l) }
                    }.vfont(14, .semibold).foregroundStyle(VUI.mint)
                }
            }
        }
    }

    /// Keeps the typed text a valid amount, one key at a time, so the same rules as the keypad apply.
    func setAmount(_ s: String) {
        var e = AmountEntry()
        for ch in s { if let d = ch.wholeNumberValue, ch.isASCII { e.digit(d) } else if ch == "." { e.point() } }
        entry = e
    }

    func load() async {
        guard let c = store.client else { message = "Add your API key in Settings first."; loaded = true; return }
        customers = (try? await c.customers()) ?? customers
        do { links = try await c.paymentLinks(); message = nil } catch { message = (error as? APIError)?.message ?? "Couldn't reach the server." }
        loaded = true
    }
    func create() {
        guard let c = store.client, let amount = entry.money, !busy else { return }
        busy = true
        Task {
            defer { busy = false }
            do {
                _ = try await c.createPaymentLink(amount: amount, description: note.trimmingCharacters(in: .whitespaces), maxUses: oneUse ? 1 : nil, customerID: customerID.isEmpty ? nil : customerID)
                entry = AmountEntry(); note = ""; oneUse = false; customerID = ""; notice = "Link created."; message = nil; Haptics.success()
                await load()
            } catch { message = (error as? APIError)?.message ?? "Couldn't create the link."; Haptics.error() }
        }
    }
    func cancel(_ l: PaymentLinkInfo) {
        guard let c = store.client else { return }
        Task {
            do { try await c.cancelPaymentLink(id: l.id); notice = "Link canceled."; message = nil; Haptics.tap(); await load() }
            catch { message = (error as? APIError)?.message ?? "Couldn't cancel the link."; Haptics.error() }
        }
    }
    func email(_ l: PaymentLinkInfo) {
        guard let c = store.client else { return }
        Task {
            do { try await c.emailPaymentLink(id: l.id); message = nil; notice = "Link emailed."; Haptics.success() }
            catch {
                let e = error as? APIError
                // the server's own text tells an operator to set an environment variable: say what it means to a merchant
                message = e?.code == "checkout_url_not_configured" ? "Emailing links is not switched on for this account yet. Ask whoever runs your Zyvor server to set the public checkout address." : (e?.message ?? "Couldn't email the link.")
                Haptics.error()
            }
        }
    }
}

/// The link's address and QR, with the system share sheet.
struct LinkShareSheet: View {
    @Environment(\.dismiss) var dismiss
    let link: PaymentLinkInfo
    let url: URL?

    var body: some View {
        ZStack {
            AmbientBackground(brand: .merchant, phase: 0.6)
            VStack(spacing: 16) {
                Text(link.amountMinor.map { Money(minor: $0).display } ?? "Payment link").vfont(26, .bold).foregroundStyle(VUI.textPrimary)
                if let url, let img = QR.image(url.absoluteString) {
                    img.resizable().frame(width: 240, height: 240).padding(14).background(.white, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                        .accessibilityLabel("QR code for the payment link")
                    Text(url.absoluteString).vfont(12).foregroundStyle(VUI.textSecondary).multilineTextAlignment(.center).textSelection(.enabled)
                    ShareLink(item: url, message: Text(link.amountMinor.map { "Pay \(Money(minor: $0).display)" } ?? "Pay here")) {
                        Label("Share link", systemImage: "square.and.arrow.up").vfont(16, .semibold).foregroundStyle(VUI.ink).padding(.horizontal, 22).padding(.vertical, 12).background(VUI.mint, in: Capsule())
                    }
                } else {
                    Text("Set your public checkout address in Settings to share this link.").vfont(14).foregroundStyle(VUI.textSecondary)
                }
                Button("Done") { dismiss() }.vfont(16).foregroundStyle(VUI.textSecondary)
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
        .presentationDetents([.large])
    }
}
