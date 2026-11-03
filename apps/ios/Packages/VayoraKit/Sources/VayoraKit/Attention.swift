import Foundation

/// How many things are waiting on the merchant right now, for a badge.
public struct Attention: Equatable, Sendable {
    public var approvals: Int
    public var reminderDrafts: Int
    /// Open disputes whose reply deadline is within `dueSoonDays` (or has passed).
    public var disputesDueSoon: Int
    public var total: Int { approvals + reminderDrafts + disputesDueSoon }
    public static let dueSoonDays = 2

    public init(approvals: Int = 0, reminderDrafts: Int = 0, disputesDueSoon: Int = 0) {
        self.approvals = approvals; self.reminderDrafts = reminderDrafts; self.disputesDueSoon = disputesDueSoon
    }

    public init(approvals: [PendingApproval], drafts: [ReminderDraft], disputes: [DisputeInfo], today: String) {
        self.init(approvals: approvals.count, reminderDrafts: drafts.count,
                  disputesDueSoon: disputes.filter { $0.isOpen && ($0.daysLeft(today: today).map { $0 <= Self.dueSoonDays } ?? false) }.count)
    }

    /// One short sentence for VoiceOver and the tab, or nil when nothing waits.
    public var summary: String? {
        var parts: [String] = []
        if approvals > 0 { parts.append("\(approvals) to approve") }
        if reminderDrafts > 0 { parts.append("\(reminderDrafts) reminder\(reminderDrafts == 1 ? "" : "s")") }
        if disputesDueSoon > 0 { parts.append("\(disputesDueSoon) dispute\(disputesDueSoon == 1 ? "" : "s") due soon") }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

extension VayoraClient {
    /// Reads the three lists. A list the key may not read counts as empty rather than failing the whole count.
    public func attention(today: String) async -> Attention {
        async let a = try? pendingApprovals(); async let d = try? reminderDrafts(); async let x = try? openDisputes()
        return Attention(approvals: await a ?? [], drafts: await d ?? [], disputes: await x ?? [], today: today)
    }
}
