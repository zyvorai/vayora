package dev.zyvor.vayora.kit

import java.time.LocalDate

enum class MandateFrequency(val raw: String, val title: String) {
    Daily("daily", "Every day"),
    Weekly("weekly", "Every week"),
    Fortnightly("fortnightly", "Every 2 weeks"),
    Monthly("monthly", "Every month"),
    Bimonthly("bimonthly", "Every 2 months"),
    Quarterly("quarterly", "Every 3 months"),
    HalfYearly("half_yearly", "Every 6 months"),
    Yearly("yearly", "Every year"),
    AsPresented("as_presented", "Whenever the merchant asks");

    companion object {
        fun fromRaw(s: String): MandateFrequency? = entries.find { it.raw == s }
    }
}

/**
 * A parsed `upi://mandate?...` link. Hostile until proven otherwise.
 * Mirrors iOS `MandatePayload`.
 */
data class MandatePayload(
    val payee: String,
    val name: String,
    val cap: Money,
    val mandateId: String,
    val frequency: MandateFrequency,
    val start: LocalDate?,
    val end: LocalDate?,
) {
    sealed class Failure : Exception() {
        data object NotMandate : Failure()
        data object MissingPayee : Failure()
        data object BadPayee : Failure()
        data object BadAmount : Failure()
        data object UnsupportedCurrency : Failure()
        data object BadMandateId : Failure()
        data object BadFrequency : Failure()
        data object BadDate : Failure()
        data object DuplicateParameter : Failure()
    }

    companion object {
        fun parse(text: String): MandatePayload {
            val trimmed = text.trim()
            if (!trimmed.lowercase().startsWith("upi://mandate?")) throw Failure.NotMandate
            val query = trimmed.substringAfter('?', "")
            val q = linkedMapOf<String, String>()
            for (item in query.split('&')) {
                if (item.isEmpty()) continue
                val eq = item.indexOf('=')
                val k = if (eq < 0) item else item.substring(0, eq)
                val v = if (eq < 0) "" else java.net.URLDecoder.decode(item.substring(eq + 1), Charsets.UTF_8)
                if (q.containsKey(k)) throw Failure.DuplicateParameter
                q[k] = v
            }
            val pa = q["pa"]?.takeIf { it.isNotEmpty() } ?: throw Failure.MissingPayee
            if (!UPIPayload.isValidVPA(pa)) throw Failure.BadPayee
            q["cu"]?.let { if (it.uppercase() != "INR") throw Failure.UnsupportedCurrency }
            val cap = q["am"]?.let { Money.parseDecimal(it) } ?: throw Failure.BadAmount
            val id = q["tn"]?.takeIf { isSafeId(it) } ?: throw Failure.BadMandateId
            val freq = q["recur"]?.let { MandateFrequency.fromRaw(it) } ?: throw Failure.BadFrequency
            val start = q["start"]?.let { parseDate(it) }
            val end = q["end"]?.let { parseDate(it) }
            if (start != null && end != null && !end.isAfter(start)) throw Failure.BadDate
            return MandatePayload(pa, q["pn"].orEmpty(), cap, id, freq, start, end)
        }

        fun isSafeId(s: String): Boolean =
            s.length in 4..64 && s.all { it.code < 128 && (it.isLetterOrDigit() || it == '_') }

        fun parseDate(s: String): LocalDate = try {
            LocalDate.parse(s)
        } catch (_: Exception) {
            throw Failure.BadDate
        }
    }
}
