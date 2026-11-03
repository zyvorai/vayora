import Foundation

/// One step in a payment's life: created, a payer tried, captured, refunded, disputed, settled.
public struct TimelineEvent: Codable, Equatable, Sendable, Identifiable {
    public let at: String
    public let kind: String
    public let title: String
    public var id: String { at + "|" + kind + "|" + title }

    public init(at: String, kind: String, title: String) { self.at = at; self.kind = kind; self.title = title }
    enum CodingKeys: String, CodingKey { case at, kind, title }   // `detail` is free-form and not shown

    /// The time of day and date in Indian time ("3 Nov, 6:19 PM"), or the raw text if it cannot be read.
    public var when: String {
        guard let d = ISO8601DateFormatter.fractional.date(from: at) ?? ISO8601DateFormatter().date(from: at) else { return at }
        let f = DateFormatter(); f.locale = Locale(identifier: "en_IN"); f.timeZone = TimeZone(identifier: "Asia/Kolkata"); f.dateFormat = "d MMM, h:mm a"
        return f.string(from: d)
    }
}

/// A payment's story, oldest first, and the server's one-line summary of where it stands.
public struct PaymentTimeline: Codable, Equatable, Sendable {
    public let standing: String
    public let events: [TimelineEvent]
    public init(standing: String, events: [TimelineEvent]) { self.standing = standing; self.events = events }
    /// Oldest first, whatever order the server sent them in (events at the same instant keep their order).
    public var ordered: [TimelineEvent] { events.enumerated().sorted { ($0.element.at, $0.offset) < ($1.element.at, $1.offset) }.map(\.element) }
}

extension ISO8601DateFormatter {
    static let fractional: ISO8601DateFormatter = { let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return f }()
}

extension VayoraClient {
    public func timeline(intentId: String) async throws -> PaymentTimeline {
        try await send(request("GET", "/payment_intents/\(intentId)/timeline"), as: PaymentTimeline.self)
    }
}
