import Foundation

/// A money mismatch someone has to look at: a settlement that does not add up, a payment the provider never confirmed, a duplicate.
public struct ReconCase: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let kind: String
    public let status: String
    public let explanation: String?
    public let assignee: String?
    public let diffMinor: Int64?
    /// What the case is about (`settlement`, `payment_attempt`, `refund`) and its id.
    public let subjectType: String?
    public let subjectID: String?
    enum CodingKeys: String, CodingKey { case id, kind, status, explanation, assignee, diffMinor = "diff_minor", subjectType = "subject_type", subjectID = "subject_id" }
    public init(id: String, kind: String, status: String = "open", explanation: String? = nil, assignee: String? = nil, diffMinor: Int64? = nil, subjectType: String? = nil, subjectID: String? = nil) {
        self.id = id; self.kind = kind; self.status = status; self.explanation = explanation; self.assignee = assignee; self.diffMinor = diffMinor; self.subjectType = subjectType; self.subjectID = subjectID
    }
    /// The settlement this case is about, when it is about one: the screen that explains it can be opened from here.
    public var settlementID: String? { subjectType == "settlement" ? subjectID : nil }
    public var isOpen: Bool { status != "resolved" }
    /// "Settlement arithmetic" from `settlement_arithmetic`.
    public var title: String { kind.replacingOccurrences(of: "_", with: " ").capitalized }
}

/// What a person must give to close a case: a note of what was found or done (the server keeps it with the case).
public enum CaseNote {
    public static func cleaned(_ s: String) -> String { s.trimmingCharacters(in: .whitespacesAndNewlines) }
    public static func isUsable(_ s: String) -> Bool { cleaned(s).count >= 3 }
}

extension VayoraClient {
    /// Cases still waiting (everything but resolved), newest first as the server sends them.
    public func openReconCases() async throws -> [ReconCase] {
        struct List: Decodable { let data: [ReconCase] }
        return try await send(request("GET", "/recon/cases"), as: List.self).data.filter(\.isOpen)
    }

    public func assignReconCase(id: String, to person: String) async throws {
        struct Done: Decodable {}
        let data = try JSONSerialization.data(withJSONObject: ["assignee": CaseNote.cleaned(person)])
        _ = try await send(request("POST", "/recon/cases/\(id)/assign", body: data), as: Done.self)
    }

    public func resolveReconCase(id: String, resolution: String) async throws {
        struct Done: Decodable {}
        let data = try JSONSerialization.data(withJSONObject: ["resolution": CaseNote.cleaned(resolution)])
        _ = try await send(request("POST", "/recon/cases/\(id)/resolve", body: data), as: Done.self)
    }
}
