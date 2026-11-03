import Foundation

/// One unpaid invoice from the ageing report.
public struct OwedInvoice: Codable, Equatable, Hashable, Sendable, Identifiable {
    public let id: String
    public let number: String
    public let dueDate: String?
    public let outstandingMinor: Int64
    public let daysPastDue: Int
    enum CodingKeys: String, CodingKey { case id, number, dueDate = "due_date", outstandingMinor = "outstanding_minor", daysPastDue = "days_past_due" }
    public init(id: String, number: String, dueDate: String? = nil, outstandingMinor: Int64, daysPastDue: Int = 0) {
        self.id = id; self.number = number; self.dueDate = dueDate; self.outstandingMinor = outstandingMinor; self.daysPastDue = daysPastDue
    }
}

/// An ageing bucket for one currency.
public struct AgeingBucket: Codable, Equatable, Sendable, Identifiable {
    public let currency: String
    public let bucket: String
    public let outstandingMinor: Int64
    public let count: Int
    public let invoices: [OwedInvoice]
    public var id: String { currency + bucket }
    enum CodingKeys: String, CodingKey { case currency, bucket, count, invoices, outstandingMinor = "outstanding_minor" }

    /// "Not yet due", "1 to 30 days late", and so on.
    public var title: String {
        switch bucket {
        case "current": return "Not yet due"
        case "1_30": return "1 to 30 days late"
        case "31_60": return "31 to 60 days late"
        case "61_90": return "61 to 90 days late"
        case "over_90": return "Over 90 days late"
        case "no_due_date": return "No due date"
        default: return bucket
        }
    }
    /// Oldest debt first, so the list reads from most to least urgent.
    public var rank: Int { ["over_90": 0, "61_90": 1, "31_60": 2, "1_30": 3, "current": 4, "no_due_date": 5][bucket] ?? 6 }
}

public struct Ageing: Codable, Equatable, Sendable {
    public let asOf: String
    public let data: [AgeingBucket]
    enum CodingKeys: String, CodingKey { case asOf = "as_of", data }
    /// Buckets in INR, most overdue first. Other currencies are left out of the phone's totals.
    public var inrBuckets: [AgeingBucket] { data.filter { $0.currency == "INR" }.sorted { $0.rank < $1.rank } }
    public var inrTotalMinor: Int64 { inrBuckets.reduce(0) { $0 + $1.outstandingMinor } }
}

/// A reminder a person has not yet approved.
public struct ReminderDraft: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let invoiceNumber: String?
    public let channel: String
    public let message: String
    enum CodingKeys: String, CodingKey { case id, channel, message, invoiceNumber = "invoice_number" }
    public init(id: String, invoiceNumber: String? = nil, channel: String = "email", message: String) { self.id = id; self.invoiceNumber = invoiceNumber; self.channel = channel; self.message = message }
}

extension VayoraClient {
    public func ageing() async throws -> Ageing { try await send(request("GET", "/collections/ageing"), as: Ageing.self) }

    public func reminderDrafts() async throws -> [ReminderDraft] {
        struct List: Decodable { let data: [ReminderDraft] }
        return try await send(request("GET", "/collections/drafts"), as: List.self).data
    }

    /// Approves a draft. Pass `message` only when the person edited the wording.
    public func approveReminderDraft(id: String, message: String? = nil) async throws {
        struct Done: Decodable {}
        // the server reads a JSON body here, and an empty body with a JSON content type is a 400: send an empty object
        let body = (try? JSONSerialization.data(withJSONObject: message.map { ["message": $0] } ?? [:])) ?? Data("{}".utf8)
        _ = try await send(request("POST", "/collections/drafts/\(id)/approve", body: body), as: Done.self)
    }

    public func rejectReminderDraft(id: String) async throws {
        struct Done: Decodable {}
        _ = try await send(request("POST", "/collections/drafts/\(id)/reject"), as: Done.self)
    }

    /// Emails an invoice to its customer (409 if it is paid, void, has no customer email, or already went today).
    public func emailInvoice(id: String) async throws {
        struct Done: Decodable {}
        _ = try await send(request("POST", "/invoices/\(id)/send"), as: Done.self)
    }

    /// A customer's promise to pay `amount` on `date` (yyyy-MM-dd).
    public func promiseToPay(invoiceID: String, amount: Money, date: String, note: String? = nil, idempotencyKey: String = UUID().uuidString) async throws {
        struct Done: Decodable {}
        var body: [String: Any] = ["promised_minor": amount.minor, "promised_date": date]
        if let note, !note.isEmpty { body["note"] = note }
        let data = try JSONSerialization.data(withJSONObject: body)
        _ = try await send(request("POST", "/invoices/\(invoiceID)/promises", body: data, idempotencyKey: idempotencyKey), as: Done.self)
    }
}
