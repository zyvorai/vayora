package dev.zyvor.vayora.kit

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZonedDateTime

object Tip {
    val percents = listOf(5, 10, 15)

    /** Tip for a base amount at a whole percent, rounded half up to the paisa. */
    fun amount(base: Long, percent: Int): Long {
        if (base <= 0 || percent <= 0) return 0
        return (base * percent + 50) / 100
    }
}

object HourlyTotals {
    fun make(payments: List<Pair<Long, Long>>, day: LocalDate, zone: ZoneId = ZoneId.systemDefault()): LongArray {
        val out = LongArray(24)
        for ((whenMillis, minor) in payments) {
            val z = Instant.ofEpochMilli(whenMillis).atZone(zone)
            if (z.toLocalDate() != day) continue
            out[z.hour] += minor
        }
        return out
    }

    fun window(totals: LongArray): IntRange {
        val first = totals.indexOfFirst { it > 0 }
        val last = totals.indexOfLast { it > 0 }
        if (first < 0 || last < 0) return 9..17
        var lo = first
        var hi = last
        while (hi - lo + 1 < 6) {
            if (lo > 0) lo--
            if (hi - lo + 1 < 6 && hi < 23) hi++
        }
        return lo..hi
    }

    fun label(hour: Int): String = when {
        hour == 0 -> "12a"
        hour < 12 -> "${hour}a"
        hour == 12 -> "12p"
        else -> "${hour - 12}p"
    }
}

object ReceiptText {
    fun make(
        merchant: String,
        amount: Money,
        status: String,
        id: String,
        whenLabel: String? = null,
        refundedMinor: Long = 0,
    ): String {
        val lines = mutableListOf(
            if (merchant.isBlank()) "Payment receipt" else "Receipt from $merchant",
            "Amount: ${amount.display}",
            "Status: ${if (status == "succeeded" || status == "captured") "Paid" else status}",
        )
        if (refundedMinor > 0) lines += "Refunded: ${Money(refundedMinor, amount.currency).display}"
        whenLabel?.takeIf { it.isNotBlank() }?.let { lines += "Date: $it" }
        lines += "Reference: $id"
        lines += "Test payment: no real money moved."
        return lines.joinToString("\n")
    }
}

class CategoryMemory {
    private val byPayee = mutableMapOf<String, String>()

    fun set(vpa: String, to: SpendCategory) {
        byPayee[vpa.lowercase()] = to.raw
    }

    fun forget(vpa: String) {
        byPayee.remove(vpa.lowercase())
    }

    fun category(forVpa: String): SpendCategory? =
        byPayee[forVpa.lowercase()]?.let { raw -> SpendCategory.entries.find { it.raw == raw } }

    fun resolve(vpa: String, name: String, note: String, transfer: Boolean): SpendCategory =
        category(forVpa = vpa) ?: SpendCategory.guess(name, note, transfer)

    fun encode(): String = byPayee.entries.joinToString(";") { "${it.key}=${it.value}" }

    companion object {
        fun decode(raw: String): CategoryMemory {
            val m = CategoryMemory()
            if (raw.isBlank()) return m
            raw.split(';').forEach {
                val p = it.split('=', limit = 2)
                if (p.size == 2) m.byPayee[p[0].lowercase()] = p[1]
            }
            return m
        }
    }
}
