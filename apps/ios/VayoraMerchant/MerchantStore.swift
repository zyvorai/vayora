import Foundation
import SwiftUI
import UserNotifications
import VayoraKit

@MainActor
final class MerchantStore: ObservableObject {
    @Published var serverURL: String { didSet { UserDefaults.standard.set(serverURL, forKey: "serverURL"); Keychain.set(serverURL, for: "serverURL") } }
    @Published var apiKey: String { didSet { Keychain.set(apiKey, for: "apiKey") } }

    /// Cashier mode hides AutoPay, Settings and refunds until the owner's PIN is entered. A shared-phone convenience, not security.
    @Published var cashierLocked: Bool { didSet { UserDefaults.standard.set(cashierLocked, forKey: "cashierLocked") } }
    @Published var regulars: RegularsBook { didSet { if let d = try? JSONEncoder().encode(regulars) { UserDefaults.standard.set(d, forKey: "regulars") } } }
    @Published var gate = PinGate()
    /// The public address of the checkout page (https), so a payment link's path can be turned into an address to share.
    @Published var checkoutURL: String { didSet { UserDefaults.standard.set(checkoutURL, forKey: "checkoutURL") } }
    @Published var closeReminder: Bool { didSet { UserDefaults.standard.set(closeReminder, forKey: "closeReminder"); syncReminder() } }
    @Published var closeMinutes: Int { didSet { UserDefaults.standard.set(closeMinutes, forKey: "closeMinutes"); syncReminder() } }
    /// Staff names for this till, who is on now, and a name on each sale made. Local records for a shift summary; not accounts.
    @Published var staff: [String] { didSet { UserDefaults.standard.set(staff, forKey: "staff") } }
    @Published var activeStaff: String? { didSet { UserDefaults.standard.set(activeStaff, forKey: "activeStaff") } }
    @Published var staffTags: RegularsBook { didSet { if let d = try? JSONEncoder().encode(staffTags) { UserDefaults.standard.set(d, forKey: "staffTags") } } }
    /// What this shop sells, and the bills that became payments (newest last, capped) so sales can be grouped by item.
    @Published var catalogue: Catalogue { didSet { if let d = try? JSONEncoder().encode(catalogue) { UserDefaults.standard.set(d, forKey: "catalogue") } } }
    @Published var billLog: [BillRecord] { didSet { if let d = try? JSONEncoder().encode(billLog) { UserDefaults.standard.set(d, forKey: "billLog") } } }
    func logBill(_ b: Bill, intentID: String) {
        guard !b.isEmpty else { return }
        billLog.append(BillRecord(intentID: intentID, date: Date(), lines: b.lines))
        if billLog.count > 500 { billLog.removeFirst(billLog.count - 500) }
    }
    /// Refunds made from this phone, with the reason and note. Local records; the server also keeps the reason.
    @Published var refundLog: [RefundEntry] { didSet { if let d = try? JSONEncoder().encode(refundLog) { UserDefaults.standard.set(d, forKey: "refundLog") } } }
    func logRefund(_ e: RefundEntry) { refundLog.append(e); if refundLog.count > 500 { refundLog.removeFirst(refundLog.count - 500) } }
    var ownerPIN: String { get { Keychain.get("ownerPIN") ?? "" } set { Keychain.set(newValue, for: "ownerPIN") } }

    /// What is waiting on the owner (approvals, reminders to approve, disputes due soon). Shown as a badge on the Owed tab and, only if
    /// the person already allowed badges, on the app icon. Not computed in cashier mode, and cleared there.
    @Published var attention = Attention()
    func refreshAttention() async {
        guard !cashierLocked, let c = client else { attention = Attention(); await Self.setIconBadge(0); return }
        let f = DateFormatter(); f.dateFormat = "yyyy-MM-dd"; f.locale = Locale(identifier: "en_US_POSIX"); f.timeZone = TimeZone(identifier: "Asia/Kolkata")
        let a = await c.attention(today: f.string(from: Date()))
        attention = a
        await Self.setIconBadge(a.total)
    }
    private static func setIconBadge(_ n: Int) async {
        let center = UNUserNotificationCenter.current()
        let st = await center.notificationSettings()
        guard st.authorizationStatus == .authorized, st.badgeSetting == .enabled else { return }   // never asks: the closing reminder owns the permission prompt
        try? await center.setBadgeCount(n)
    }

    func syncReminder() { Task { await ClosingReminder.sync(enabled: closeReminder, minutes: closeMinutes) } }

    init() {
        let d = UserDefaults.standard
        cashierLocked = d.bool(forKey: "cashierLocked")
        checkoutURL = d.string(forKey: "checkoutURL") ?? ""
        regulars = (d.data(forKey: "regulars")).flatMap { try? JSONDecoder().decode(RegularsBook.self, from: $0) } ?? RegularsBook()
        catalogue = (d.data(forKey: "catalogue")).flatMap { try? JSONDecoder().decode(Catalogue.self, from: $0) } ?? Catalogue()
        billLog = (d.data(forKey: "billLog")).flatMap { try? JSONDecoder().decode([BillRecord].self, from: $0) } ?? []
        refundLog = (d.data(forKey: "refundLog")).flatMap { try? JSONDecoder().decode([RefundEntry].self, from: $0) } ?? []
        staff = d.stringArray(forKey: "staff") ?? []
        activeStaff = d.string(forKey: "activeStaff")
        staffTags = (d.data(forKey: "staffTags")).flatMap { try? JSONDecoder().decode(RegularsBook.self, from: $0) } ?? RegularsBook()
        closeReminder = d.bool(forKey: "closeReminder")
        closeMinutes = d.object(forKey: "closeMinutes") as? Int ?? (21 * 60)
        let saved = Keychain.get("serverURL") ?? UserDefaults.standard.string(forKey: "serverURL") ?? "http://127.0.0.1:8080"
        serverURL = saved
        // an address saved by an older build lives only in UserDefaults: copy it to the Keychain so a reinstall keeps it
        if URL(string: saved)?.host != "127.0.0.1", Keychain.get("serverURL") == nil { Keychain.set(saved, for: "serverURL") }
        apiKey = Keychain.get("apiKey") ?? ""
        #if DEBUG
        // UI tests only (a debug build): point the app at a test server without typing into Settings. Never compiled into a release build.
        if let u = d.string(forKey: "uitestServer"), let k = d.string(forKey: "uitestKey"), k.hasPrefix("zk_test") { serverURL = u; apiKey = k }
        #endif
    }

    /// Built-in demo bank: no server, no key. Stored per phone.
    @Published var demoMode: Bool = UserDefaults.standard.bool(forKey: "demoMode") { didSet { UserDefaults.standard.set(demoMode, forKey: "demoMode") } }

    var client: VayoraClient? {
        if demoMode { return .demo() }
        guard !apiKey.isEmpty, let u = URL(string: serverURL) else { return nil }
        return VayoraClient(baseURL: u, apiKey: apiKey)
    }
}
