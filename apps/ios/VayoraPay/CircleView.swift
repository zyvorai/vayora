import SwiftUI
import VayoraKit
import VayoraUI

/// Circle (test mode): lend this wallet to someone up to a limit, or pay from a circle someone shared with you. The simulated bank keeps the books.
struct CircleView: View {
    @EnvironmentObject var store: Store
    @State private var creating = false
    @State private var scanning = false
    @State private var showing: CircleRecord?
    @State private var message: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                GlassCard {
                    VStack(alignment: .leading, spacing: 10) {
                        Label("Lend my account", systemImage: "person.2.badge.key.fill").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                        ForEach(store.circles.filter { $0.role == .primary }) { c in card(c) }
                        if store.circles.filter({ $0.role == .primary }).isEmpty { Text("Let someone you trust pay from your wallet, up to a limit you set. You can take it back any time.").vfont(14).foregroundStyle(VUI.textSecondary) }
                        Button { creating = true } label: { Label("New circle", systemImage: "plus.circle.fill") }.vfont(15, .semibold).foregroundStyle(VUI.violet)
                    }
                }
                GlassCard {
                    VStack(alignment: .leading, spacing: 10) {
                        Label("I can pay from", systemImage: "creditcard.and.123").vfont(17, .semibold).foregroundStyle(VUI.textPrimary)
                        ForEach(store.circles.filter { $0.role == .delegate }) { c in card(c) }
                        if store.circles.filter({ $0.role == .delegate }).isEmpty { Text("Scan a circle QR from someone who shared their account with you. Then pick it on the review screen when you pay a merchant.").vfont(14).foregroundStyle(VUI.textSecondary) }
                        Button { scanning = true } label: { Label("Scan a circle QR", systemImage: "qrcode.viewfinder") }.vfont(15, .semibold).foregroundStyle(VUI.violet)
                    }
                }
                if let message { Text(message).vfont(13).foregroundStyle(VUI.textSecondary) }
                Text("Test mode: the simulator plays the bank. Real UPI Circle has its own rules, which are not verified here. A circle payment comes out of the lender's wallet, not yours.").vfont(12).foregroundStyle(VUI.textTertiary)
            }
            .padding(20)
        }
        .background(VUI.ink)
        .refreshable { await store.refreshCircles() }
        .task { await store.refreshCircles() }
        .sheet(isPresented: $creating) { NewCircleSheet { r in showing = r }.environmentObject(store) }
        .sheet(item: $showing) { r in CircleQRSheet(record: r).environmentObject(store) }
        .fullScreenCover(isPresented: $scanning) {
            ScannerView(brand: .pay, hint: "Scan the circle QR") { text in
                scanning = false
                guard let link = try? CirclePayload.parse(text) else { message = "That is not a circle QR."; return }
                Task { message = await store.joinCircle(link) ?? "Joined \(link.primary)'s circle."; await store.refreshCircles() }
            } onCancel: { scanning = false }
        }
    }

    func card(_ c: CircleRecord) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(c.role == .primary ? c.other : "\(c.other)'s account").vfont(16, .semibold).foregroundStyle(VUI.textPrimary); Spacer()
                StatusPill(c.isLive ? "Active" : "Ended", c.isLive ? .success : .neutral)
            }
            Text("Up to \(Money(minor: c.perPaymentMinor).display) a payment · \(Money(minor: c.remainingMinor).display) of \(Money(minor: c.totalMinor).display) left").vfont(13).foregroundStyle(VUI.textSecondary)
            HStack(spacing: 18) {
                if c.role == .primary, c.isLive { Button("Show QR") { showing = c }.foregroundStyle(VUI.violet) }
                if c.isLive || c.role == .delegate {
                    Button(c.role == .primary ? "Take back" : "Leave", role: .destructive) { Task { if !(await store.endCircle(c.id)) { message = "Couldn't do that. Check the test server." } } }.foregroundStyle(VUI.coral)
                }
            }.vfont(14, .semibold)
        }
    }
}

struct NewCircleSheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    var onCreated: (CircleRecord) -> Void
    @State private var name = ""
    @State private var per = ""
    @State private var total = ""
    @State private var problem: String?
    @State private var working = false

    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.9)
            VStack(alignment: .leading, spacing: 16) {
                HStack { Text("New circle").vfont(24, .bold).foregroundStyle(VUI.textPrimary); Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                GlassCard {
                    VStack(alignment: .leading, spacing: 12) {
                        field("Who can pay (name)", $name, .default)
                        field("Most per payment, in ₹", $per, .decimalPad)
                        field("Most in total, in ₹", $total, .decimalPad)
                        if let problem { Text(problem).vfont(13).foregroundStyle(VUI.coral) }
                        Text("Their payments come out of your test wallet. Nothing is shared except a QR they scan.").vfont(12).foregroundStyle(VUI.textTertiary)
                    }
                }
                Button(action: create) { Text(working ? "Creating…" : "Create").vfont(18, .semibold).foregroundStyle(.white).frame(maxWidth: .infinity).padding(.vertical, 17).background(Brand.pay.gradient, in: RoundedRectangle(cornerRadius: 24, style: .continuous)) }
                    .buttonStyle(PressScale()).opacity(valid && !working ? 1 : 0.4).disabled(!valid || working)
                Spacer()
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
    var valid: Bool { !name.trimmingCharacters(in: .whitespaces).isEmpty && Money(decimal: per) != nil && Money(decimal: total) != nil }
    func field(_ t: String, _ b: Binding<String>, _ k: UIKeyboardType) -> some View {
        TextField(t, text: b).keyboardType(k).foregroundStyle(VUI.textPrimary).padding(12).background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
    func create() {
        guard let p = Money(decimal: per), let t = Money(decimal: total) else { return }
        guard p.minor <= t.minor else { problem = "The total can't be less than one payment."; Haptics.error(); return }
        working = true
        Task {
            defer { working = false }
            switch await store.createCircle(delegate: name.trimmingCharacters(in: .whitespaces), perPayment: p, total: t) {
            case .success(let r): Haptics.success(); dismiss(); onCreated(r)
            case .failure(let e): problem = e.message; Haptics.error()
            }
        }
    }
}

struct CircleQRSheet: View {
    @EnvironmentObject var store: Store
    @Environment(\.dismiss) var dismiss
    let record: CircleRecord
    var link: String { CirclePayload.make(id: record.id, primary: store.myName.isEmpty ? "Me" : store.myName, perPayment: Money(minor: record.perPaymentMinor), total: Money(minor: record.totalMinor)) }
    var body: some View {
        ZStack {
            AmbientBackground(brand: .pay, phase: 0.3)
            VStack(spacing: 16) {
                HStack { Text("\(record.other) scans this").vfont(22, .bold).foregroundStyle(VUI.textPrimary); Spacer()
                    Button { dismiss() } label: { Image(systemName: "xmark").font(.system(size: 15, weight: .bold)).foregroundStyle(VUI.textPrimary).frame(width: 36, height: 36).background(.white.opacity(0.14), in: Circle()) }.accessibilityLabel("Close") }
                Spacer()
                QR.image(link)?.interpolation(.none).resizable().scaledToFit().frame(maxWidth: 280).padding(16).background(.white, in: RoundedRectangle(cornerRadius: 24, style: .continuous)).accessibilityLabel("Circle QR")
                Text("Up to \(Money(minor: record.perPaymentMinor).display) a payment, \(Money(minor: record.totalMinor).display) in total").vfont(14).foregroundStyle(VUI.textSecondary)
                Spacer()
                Text("Test mode. Their phone must reach the same test server.").vfont(12).foregroundStyle(VUI.textTertiary)
            }.padding(24)
        }
        .environment(\.colorScheme, .dark)
    }
}
