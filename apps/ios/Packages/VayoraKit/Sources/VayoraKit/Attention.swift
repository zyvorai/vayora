import Foundation

/// How many things are waiting on the merchant right now, for a badge.
public struct Attention: Equatable, Sendable {
    public var approvals: Int
    public var reminderDrafts: Int
    /// Open disputes whose reply deadline is within `dueSoonDays` (or has passed).
    public var disputesDueSoon: Int
    /// Approved payouts whose transfer has not been recorded, and reconciliation cases nobody has closed.
    public var payoutsToPay: Int
    public var reconCases: Int
    public var total: Int { approvals + reminderDrafts + disputesDueSoon + payoutsToPay + reconCases }
    public static let dueSoonDays = 2

    public init(approvals: Int = 0, reminderDrafts: Int = 0, disputesDueSoon: Int = 0, payoutsToPay: Int = 0, reconCases: Int = 0) {
        self.approvals = approvals; self.reminderDrafts = reminderDrafts; self.disputesDueSoon = disputesDueSoon
        self.payoutsToPay = payoutsToPay; self.reconCases = reconCases
    }

    public init(approvals: [PendingApproval], drafts: [ReminderDraft], disputes: [DisputeInfo], today: String, payouts: [ApprovedPayout] = [], cases: [ReconCase] = []) {
        self.init(approvals: approvals.count, reminderDrafts: drafts.count,
                  disputesDueSoon: disputes.filter { $0.isOpen && ($0.daysLeft(today: today).map { $0 <= Self.dueSoonDays } ?? false) }.count,
                  payoutsToPay: payouts.count, reconCases: cases.filter(\.isOpen).count)
    }

    /// One short sentence for VoiceOver and the tab, or nil when nothing waits.
    public var summary: String? {
        var parts: [String] = []
        if approvals > 0 { parts.append("\(approvals) to approve") }
        if reminderDrafts > 0 { parts.append("\(reminderDrafts) reminder\(reminderDrafts == 1 ? "" : "s")") }
        if disputesDueSoon > 0 { parts.append("\(disputesDueSoon) dispute\(disputesDueSoon == 1 ? "" : "s") due soon") }
        if payoutsToPay > 0 { parts.append("\(payoutsToPay) payout\(payoutsToPay == 1 ? "" : "s") to pay") }
        if reconCases > 0 { parts.append("\(reconCases) case\(reconCases == 1 ? "" : "s") to check") }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

extension VayoraClient {
    /// Reads the lists that need a person. A list the key may not read counts as empty rather than failing the whole count.
    public func attention(today: String) async -> Attention {
        async let a = try? pendingApprovals(); async let d = try? reminderDrafts(); async let x = try? openDisputes()
        async let p = try? approvedPayouts(); async let r = try? openReconCases()
        return Attention(approvals: await a ?? [], drafts: await d ?? [], disputes: await x ?? [], today: today, payouts: await p ?? [], cases: await r ?? [])
    }
}
