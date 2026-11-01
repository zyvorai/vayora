import AppIntents
import Foundation
import VayoraKit

/// "How much did I spend in Vayora Pay": answered from this phone's saved history, in the Shortcuts app or by voice.
struct SpendThisMonthIntent: AppIntent {
    static var title: LocalizedStringResource = "Spend this month"
    static var description = IntentDescription("Says how much you paid this month in Vayora Pay. Test money.")
    static var openAppWhenRun = false

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let history = (UserDefaults.standard.data(forKey: "history")).flatMap { try? JSONDecoder().decode([Txn].self, from: $0) } ?? []
        let entries = history.map { SpendEntry(amountMinor: $0.amountMinor, category: SpendCategory(rawValue: $0.category ?? "") ?? .other, date: $0.date, paid: $0.status == .success) }
        let total = Money(minor: MonthSummary.total(entries, now: Date()))
        return .result(dialog: "You paid \(total.display) this month. That is test money.")
    }
}

struct TestBalanceIntent: AppIntent {
    static var title: LocalizedStringResource = "Test balance"
    static var description = IntentDescription("Says your Vayora Pay test balance.")
    static var openAppWhenRun = false

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let minor = UserDefaults.standard.object(forKey: "balance") as? Int64 ?? Store.startingBalance
        return .result(dialog: "Your test balance is \(Money(minor: minor).display). It is not a bank balance.")
    }
}

struct VayoraPayShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: SpendThisMonthIntent(), phrases: ["How much did I spend in \(.applicationName)", "My spending in \(.applicationName)"], shortTitle: "Spend this month", systemImageName: "chart.bar.fill")
        AppShortcut(intent: TestBalanceIntent(), phrases: ["My balance in \(.applicationName)", "Test balance in \(.applicationName)"], shortTitle: "Test balance", systemImageName: "indianrupeesign.circle")
    }
}
