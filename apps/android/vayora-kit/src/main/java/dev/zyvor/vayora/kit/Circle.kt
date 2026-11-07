package dev.zyvor.vayora.kit

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import java.net.URI
import java.net.URLDecoder
import java.net.URLEncoder

/** `vayora://circle?id=…&pn=…&per=…&total=…` */
object CirclePayload {
    sealed class Failure : Exception() {
        data object NotCircle : Failure()
        data object BadId : Failure()
        data object BadName : Failure()
        data object BadAmount : Failure()
        data object DuplicateParameter : Failure()
    }

    data class Link(
        val id: String,
        val primary: String,
        val perPayment: Money,
        val total: Money,
    )

    fun make(id: String, primary: String, perPayment: Money, total: Money): String {
        fun enc(s: String) = URLEncoder.encode(s, Charsets.UTF_8).replace("+", "%20")
        return "vayora://circle?id=${enc(id)}&pn=${enc(primary)}&per=${enc(perPayment.decimalString)}&total=${enc(total.decimalString)}"
    }

    fun parse(text: String): Link {
        val t = text.trim()
        val uri = try {
            URI(t)
        } catch (_: Exception) {
            throw Failure.NotCircle
        }
        if (uri.scheme?.lowercase() != "vayora" || uri.host?.lowercase() != "circle") throw Failure.NotCircle
        val q = linkedMapOf<String, String>()
        for (item in (uri.rawQuery ?: "").split('&')) {
            if (item.isEmpty()) continue
            val eq = item.indexOf('=')
            val k = if (eq < 0) item else item.substring(0, eq)
            val v = if (eq < 0) "" else URLDecoder.decode(item.substring(eq + 1), Charsets.UTF_8)
            if (q.containsKey(k)) throw Failure.DuplicateParameter
            q[k] = v
        }
        val id = q["id"]?.takeIf { MandatePayload.isSafeId(it) } ?: throw Failure.BadId
        val name = q["pn"].orEmpty().trim()
        if (name.isEmpty() || name.length > 60 || name.any { it == '\n' || it == '\r' }) throw Failure.BadName
        val per = q["per"]?.let { Money.parseDecimal(it) } ?: throw Failure.BadAmount
        val total = q["total"]?.let { Money.parseDecimal(it) } ?: throw Failure.BadAmount
        if (per.minor > total.minor) throw Failure.BadAmount
        return Link(id, name, per, total)
    }
}

enum class CircleRole { Primary, Delegate }

data class CircleRecord(
    val id: String,
    val role: CircleRole,
    val other: String,
    val perPaymentMinor: Long,
    val totalMinor: Long,
    val status: String = "active",
    val remainingMinor: Long = totalMinor,
    val appliedIds: Set<String> = emptySet(),
) {
    val isLive: Boolean get() = status == "active"
    fun canCover(amountMinor: Long): Boolean =
        role == CircleRole.Delegate && isLive && amountMinor > 0 &&
            amountMinor <= perPaymentMinor && amountMinor <= remainingMinor
}

@Serializable
data class PayerDebit(
    @SerialName("attempt_id") val attemptId: String,
    @SerialName("amount_minor") val amountMinor: Long,
    val at: String = "",
)

@Serializable
data class CircleInfo(
    @SerialName("circle_id") val circleId: String,
    val primary: String,
    val delegate: String,
    val status: String,
    @SerialName("max_per_payment_minor") val maxPerPaymentMinor: Long,
    @SerialName("total_limit_minor") val totalLimitMinor: Long,
    @SerialName("used_minor") val usedMinor: Long = 0,
    @SerialName("remaining_minor") val remainingMinor: Long,
    val spends: List<PayerDebit> = emptyList(),
)

@Serializable
data class CircleCreated(@SerialName("circle_id") val circleId: String)

@Serializable
data class CirclePayResult(
    val status: String,
    @SerialName("remaining_minor") val remainingMinor: Long = 0,
)
