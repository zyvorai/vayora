import Foundation
import UserNotifications
import VayoraKit

/// Monthly local notifications for bill reminders. They remind; they never pay.
enum ReminderScheduler {
    static func sync(_ reminders: [BillReminder]) async {
        let center = UNUserNotificationCenter.current()
        center.removeAllPendingNotificationRequests()
        let on = reminders.filter(\.enabled)
        guard !on.isEmpty else { return }
        guard (try? await center.requestAuthorization(options: [.alert, .sound])) == true else { return }
        for r in on {
            let content = UNMutableNotificationContent()
            content.title = "\(r.name) is due"
            content.body = "\(Money(minor: r.amountMinor).display). A test reminder: nothing is paid for you."
            var when = DateComponents(); when.day = r.dayOfMonth; when.hour = 9; when.minute = 0
            let trigger = UNCalendarNotificationTrigger(dateMatching: when, repeats: true)
            try? await center.add(UNNotificationRequest(identifier: r.id.uuidString, content: content, trigger: trigger))
        }
    }
}
