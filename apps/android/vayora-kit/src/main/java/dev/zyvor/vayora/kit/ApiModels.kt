package dev.zyvor.vayora.kit

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

data class ApiError(val status: Int, val code: String, override val message: String) : Exception(message)

@Serializable
data class PaymentIntent(
    val id: String,
    @SerialName("amount_minor") val amountMinor: Long,
    val currency: String,
    val status: String,
    @SerialName("client_secret") val clientSecret: String? = null,
    @SerialName("created_at") val createdAt: String? = null,
) {
    val money: Money get() = Money(amountMinor, currency)
}

@Serializable
data class ClientActionInfo(val type: String, val uri: String? = null)

@Serializable
data class AttemptInfo(
    val id: String,
    val status: String,
    @SerialName("failure_reason") val failureReason: String? = null,
    @SerialName("client_action") val clientAction: ClientActionInfo? = null,
    @SerialName("amount_minor") val amountMinor: Long? = null,
    @SerialName("refunded_minor") val refundedMinor: Long? = null,
    @SerialName("reserved_refund_minor") val reservedRefundMinor: Long? = null,
) {
    val upiURI: String?
        get() {
            val a = clientAction ?: return null
            val u = a.uri ?: return null
            return if (a.type == "upi_intent" && u.lowercase().startsWith("upi://")) u else null
        }
    val isPaid: Boolean get() = status == "captured"
    val isFinal: Boolean get() = status in setOf("captured", "failed", "canceled")
}

@Serializable
data class RefundInfo(
    val id: String,
    val status: String,
    @SerialName("amount_minor") val amountMinor: Long,
)

@Serializable
data class SimPayResult(val status: String)

@Serializable
private data class ErrorEnvelope(val error: Body) {
    @Serializable
    data class Body(val code: String, val message: String)
}

@Serializable
data class PagePaymentIntents(val data: List<PaymentIntent> = emptyList())

internal fun parseApiError(status: Int, body: String): ApiError {
    return try {
        val env = json.decodeFromString(ErrorEnvelope.serializer(), body)
        ApiError(status, env.error.code, env.error.message)
    } catch (_: Exception) {
        ApiError(status, "unknown", body.ifBlank { "HTTP $status" })
    }
}
