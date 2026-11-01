import Foundation
import UserNotifications

/// A daily local notification at closing time. Its text is fixed when it is scheduled, so it asks you to open the app; it cannot carry today's total.
enum ClosingReminder {
    static let id = "closing-reminder"
    static func sync(enabled: Bool, minutes: Int) async {
        let center = UNUserNotificationCenter.current()
        center.removePendingNotificationRequests(withIdentifiers: [id])
        guard enabled, (try? await center.requestAuthorization(options: [.alert, .sound])) == true else { return }
        let c = UNMutableNotificationContent()
        c.title = "Time to close the day"
        c.body = "Open Vayora Merchant to see today's takings and close the till. A test reminder."
        var when = DateComponents(); when.hour = minutes / 60; when.minute = minutes % 60
        try? await center.add(UNNotificationRequest(identifier: id, content: c, trigger: UNCalendarNotificationTrigger(dateMatching: when, repeats: true)))
    }
}
