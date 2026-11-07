package dev.zyvor.vayora.kit

/**
 * A parsed `upi://pay?...` payload. Anything scanned from a QR is hostile until proven otherwise.
 * Mirrors `VayoraKit.UPIPayload` on iOS.
 */
data class UPIPayload(
    val payee: String,
    val name: String,
    val amount: Money?,
    val note: String,
) {
    sealed class Failure : Exception() {
        data object NotUPI : Failure()
        data object MissingPayee : Failure()
        data object BadPayee : Failure()
        data object BadAmount : Failure()
        data object UnsupportedCurrency : Failure()
    }

    companion object {
        fun parse(text: String): UPIPayload {
            val trimmed = text.trim()
            if (!trimmed.lowercase().startsWith("upi://pay?")) throw Failure.NotUPI
            val query = trimmed.substringAfter('?', "")
            val q = linkedMapOf<String, String>()
            for (item in query.split('&')) {
                if (item.isEmpty()) continue
                val eq = item.indexOf('=')
                val k = if (eq < 0) item else item.substring(0, eq)
                val v = if (eq < 0) "" else item.substring(eq + 1)
                if (q.containsKey(k)) throw Failure.BadAmount
                q[k] = java.net.URLDecoder.decode(v, Charsets.UTF_8)
            }
            val pa = q["pa"]?.takeIf { it.isNotEmpty() } ?: throw Failure.MissingPayee
            if (!isValidVPA(pa)) throw Failure.BadPayee
            q["cu"]?.let { if (it.uppercase() != "INR") throw Failure.UnsupportedCurrency }
            val amount = q["am"]?.takeIf { it.isNotEmpty() }?.let {
                Money.parseDecimal(it) ?: throw Failure.BadAmount
            }
            return UPIPayload(pa, q["pn"].orEmpty(), amount, q["tn"].orEmpty())
        }

        /** handle@bank: letters, digits, `.`, `-`, `_` on both sides, one `@`, a sane length. */
        fun isValidVPA(v: String): Boolean {
            val parts = v.split('@')
            if (parts.size != 2 || v.length > 80) return false
            fun ok(s: String) = s.isNotEmpty() && s.all {
                it.code < 128 && (it.isLetterOrDigit() || it in ".-_")
            }
            return ok(parts[0]) && ok(parts[1])
        }
    }
}
