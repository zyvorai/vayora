package dev.zyvor.vayora.kit

/**
 * An amount in integer minor units (paise). Never a floating point number: ₹499.00 is `49900`.
 * Mirrors `VayoraKit.Money` on iOS.
 */
data class Money(val minor: Long, val currency: String = "INR") : Comparable<Money> {
    init {
        require(currency.isNotBlank())
    }

    override fun compareTo(other: Money): Int = minor.compareTo(other.minor)

    /** "499.00", for UPI payloads and APIs that want a decimal string. */
    val decimalString: String
        get() {
            val sign = if (minor < 0) "-" else ""
            val a = kotlin.math.abs(minor)
            return "$sign${a / 100}.${(a % 100).toString().padStart(2, '0')}"
        }

    /** "₹499.00", with Indian digit grouping (1,23,456.00). */
    val display: String
        get() {
            val rupees = (kotlin.math.abs(minor) / 100).toString()
            val grouped = buildString {
                rupees.reversed().forEachIndexed { i, ch ->
                    val atGroupEdge = i == 3 || (i > 3 && (i - 3) % 2 == 0)
                    if (atGroupEdge) append(',')
                    append(ch)
                }
            }.reversed()
            val symbol = if (currency == "INR") "₹" else "$currency "
            val sign = if (minor < 0) "-" else ""
            return "$sign$symbol$grouped.${(kotlin.math.abs(minor) % 100).toString().padStart(2, '0')}"
        }

    companion object {
        /**
         * Parses what a person types or a UPI `am` field carries: "499", "499.5", "499.50".
         * Rejects negatives, zero, more than two decimals, exponents and non-digits.
         */
        fun parseDecimal(text: String, currency: String = "INR"): Money? {
            val parts = text.split('.', limit = 2)
            val whole = parts.firstOrNull() ?: return null
            if (whole.isEmpty() || !whole.all { it.isDigit() }) return null
            var frac = if (parts.size == 2) parts[1] else ""
            if (parts.size == 2 && frac.isEmpty()) return null
            if (frac.length > 2 || !frac.all { it.isDigit() }) return null
            while (frac.length < 2) frac += "0"
            val w = whole.toLongOrNull() ?: return null
            if (w > 10_000_000_000L) return null
            val f = frac.toLongOrNull() ?: return null
            val total = w * 100 + f
            if (total <= 0) return null
            return Money(total, currency)
        }
    }
}
