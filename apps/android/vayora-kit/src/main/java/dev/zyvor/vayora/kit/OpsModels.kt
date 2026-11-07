package dev.zyvor.vayora.kit

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class OwedInvoice(
    val id: String,
    val number: String,
    @SerialName("due_date") val dueDate: String? = null,
    @SerialName("outstanding_minor") val outstandingMinor: Long,
    @SerialName("days_past_due") val daysPastDue: Int = 0,
)

@Serializable
data class AgeingBucket(
    val currency: String,
    val bucket: String,
    @SerialName("outstanding_minor") val outstandingMinor: Long,
    val count: Int,
    val invoices: List<OwedInvoice> = emptyList(),
) {
    val title: String
        get() = when (bucket) {
            "current" -> "Not yet due"
            "1_30" -> "1 to 30 days late"
            "31_60" -> "31 to 60 days late"
            "61_90" -> "61 to 90 days late"
            "over_90" -> "Over 90 days late"
            "no_due_date" -> "No due date"
            else -> bucket
        }
    val rank: Int
        get() = mapOf(
            "over_90" to 0, "61_90" to 1, "31_60" to 2, "1_30" to 3, "current" to 4, "no_due_date" to 5,
        )[bucket] ?: 6
}

@Serializable
data class Ageing(
    @SerialName("as_of") val asOf: String,
    val data: List<AgeingBucket> = emptyList(),
) {
    val inrBuckets: List<AgeingBucket>
        get() = data.filter { it.currency == "INR" }.sortedBy { it.rank }
    val inrTotalMinor: Long get() = inrBuckets.sumOf { it.outstandingMinor }
}

@Serializable
data class ReminderDraft(
    val id: String,
    @SerialName("invoice_number") val invoiceNumber: String? = null,
    val channel: String = "email",
    val message: String,
)

@Serializable
data class ReminderDraftPage(val data: List<ReminderDraft> = emptyList())

@Serializable
data class RefundRow(
    val id: String,
    @SerialName("amount_minor") val amountMinor: Long,
    val currency: String = "INR",
    val status: String,
    val reason: String? = null,
    @SerialName("requested_by") val requestedBy: String = "",
)

@Serializable
data class PayoutRow(
    val id: String,
    @SerialName("amount_minor") val amountMinor: Long,
    val currency: String = "INR",
    val status: String,
    val purpose: String? = null,
    @SerialName("requested_by") val requestedBy: String = "",
)

@Serializable
data class RefundPage(val data: List<RefundRow> = emptyList())

@Serializable
data class PayoutPage(val data: List<PayoutRow> = emptyList())

data class PendingApproval(
    val kind: Kind,
    val rawId: String,
    val amountMinor: Long,
    val currency: String = "INR",
    val detail: String = "",
    val requestedBy: String = "",
) {
    enum class Kind { Refund, Payout }
    val id: String get() = "${kind.name.lowercase()}:$rawId"
    val title: String get() = if (kind == Kind.Refund) "Refund" else "Payout"
}

@Serializable
data class CashflowWeek(
    val starts: String,
    @SerialName("due_minor") val dueMinor: Long,
)

@Serializable
data class Cashflow(
    @SerialName("as_of") val asOf: String? = null,
    val weeks: List<CashflowWeek> = emptyList(),
    @SerialName("overdue_minor") val overdueMinor: Long = 0,
    @SerialName("due_later_minor") val dueLaterMinor: Long = 0,
    @SerialName("no_due_date_minor") val noDueDateMinor: Long = 0,
    @SerialName("promised_minor") val promisedMinor: Long = 0,
    @SerialName("payouts_waiting_minor") val payoutsWaitingMinor: Long = 0,
    @SerialName("daily_receipts_run_rate_minor") val dailyReceiptsRunRateMinor: Long = 0,
) {
    val totalOwedMinor: Long
        get() = overdueMinor + weeks.sumOf { it.dueMinor } + dueLaterMinor + noDueDateMinor
}

@Serializable
data class SettlementInfo(
    val id: String,
    val provider: String,
    val utr: String? = null,
    @SerialName("gross_minor") val grossMinor: Long,
    @SerialName("refunds_minor") val refundsMinor: Long = 0,
    @SerialName("fee_minor") val feeMinor: Long = 0,
    @SerialName("adjustment_minor") val adjustmentMinor: Long = 0,
    @SerialName("net_minor") val netMinor: Long,
    @SerialName("settled_on") val settledOn: String? = null,
    val status: String = "matched",
)

@Serializable
data class SettlementPage(val data: List<SettlementInfo> = emptyList())

@Serializable
data class GstTotals(
    @SerialName("taxable_minor") val taxableMinor: Long = 0,
    @SerialName("cgst_minor") val cgstMinor: Long = 0,
    @SerialName("sgst_minor") val sgstMinor: Long = 0,
    @SerialName("igst_minor") val igstMinor: Long = 0,
)

@Serializable
data class GstSummary(
    val period: String? = null,
    val totals: GstTotals? = null,
) {
    val taxableMinor: Long get() = totals?.taxableMinor ?: 0
    val cgstMinor: Long get() = totals?.cgstMinor ?: 0
    val sgstMinor: Long get() = totals?.sgstMinor ?: 0
    val igstMinor: Long get() = totals?.igstMinor ?: 0
    val taxTotalMinor: Long get() = cgstMinor + sgstMinor + igstMinor
}

@Serializable
data class MandateInfo(
    val id: String,
    val status: String,
    @SerialName("max_amount_minor") val maxAmountMinor: Long? = null,
    val frequency: String? = null,
    @SerialName("customer_id") val customerId: String? = null,
)

@Serializable
data class MandatePage(val data: List<MandateInfo> = emptyList())

@Serializable
data class DisputeRow(
    val id: String,
    @SerialName("amount_minor") val amountMinor: Long,
    val currency: String = "INR",
    val status: String,
    val reason: String? = null,
    @SerialName("respond_by") val respondBy: String? = null,
)

@Serializable
data class DisputePage(val data: List<DisputeRow> = emptyList())

@Serializable
data class ReconCase(
    val id: String,
    val kind: String? = null,
    @SerialName("gap_minor") val gapMinor: Long = 0,
    val summary: String? = null,
    val status: String = "open",
)

@Serializable
data class ReconCasePage(val data: List<ReconCase> = emptyList())

@Serializable
data class BranchInfo(val id: String, val name: String)

@Serializable
data class BranchPage(val data: List<BranchInfo> = emptyList())

@Serializable
data class ClosingDigital(
    val method: String,
    val payments: Int = 0,
    @SerialName("net_minor") val netMinor: Long = 0,
)

@Serializable
data class ClosingInfo(
    val id: String,
    @SerialName("expected_cash_minor") val expectedCashMinor: Long,
    @SerialName("counted_cash_minor") val countedCashMinor: Long,
    @SerialName("difference_minor") val differenceMinor: Long,
    val digital: List<ClosingDigital> = emptyList(),
    val status: String = "pending_signoff",
) {
    val verdict: String
        get() = when {
            differenceMinor == 0L -> "Matches"
            differenceMinor < 0L -> "Short"
            else -> "Over"
        }
}
