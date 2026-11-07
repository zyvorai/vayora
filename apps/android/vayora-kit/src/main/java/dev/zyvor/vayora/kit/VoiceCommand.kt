package dev.zyvor.vayora.kit

/** Spoken words → a payment to review. Only proposes saved payees; never auto-pays. */
object VoiceCommand {
    sealed class Result {
        data class Pay(val payee: Payee, val amount: Money) : Result()
        data class Ambiguous(val payees: List<Payee>, val amount: Money) : Result()
        data class NoPayee(val name: String) : Result()
        data object NoAmount : Result()
        data object NotAPayment : Result()
    }

    private val verbs = setOf(
        "pay", "send", "transfer", "give", "भेजो", "भेजें", "भेज", "दो", "bhejo", "bhej", "do", "दे", "de",
    )
    private val fillers = setOf(
        "rupees", "rupee", "rs", "inr", "₹", "रुपये", "रुपया", "रुपए", "rupaye", "rupay",
        "to", "को", "ko", "please", "the", "के", "ke", "लिए", "liye", "for", "and", "paise", "पैसे",
    )

    private fun ascii(s: String): String {
        val map = mapOf(
            '०' to '0', '१' to '1', '२' to '2', '३' to '3', '४' to '4',
            '५' to '5', '६' to '6', '७' to '7', '८' to '8', '९' to '9',
        )
        return buildString { for (c in s) append(map[c] ?: c) }
    }

    private fun Char.isDevanagari(): Boolean = code in 0x0900..0x097F

    private fun tokenize(text: String): List<String> {
        val out = mutableListOf<String>()
        val buf = StringBuilder()
        fun flush() {
            if (buf.isNotEmpty()) {
                out += buf.toString()
                buf.clear()
            }
        }
        for (c in text) {
            val keep = c.isLetter() || c.isDigit() || c == '.' || c == '₹' || c.isDevanagari()
            if (keep) buf.append(c) else flush()
        }
        flush()
        return out
    }

    fun parse(spoken: String, payees: List<Payee>): Result {
        val text = ascii(spoken).lowercase().replace(",", "").replace("₹", " ₹ ")
        val words = tokenize(text)
        if (words.none { it in verbs }) return Result.NotAPayment

        var amount: Money? = null
        var amountAt: Int? = null
        for ((i, w) in words.withIndex()) {
            if (w.firstOrNull()?.isDigit() != true) continue
            if (!w.all { it.isDigit() || it == '.' }) continue
            val m = Money.parseDecimal(w) ?: continue
            if (m.minor in 1..100_000_00_00L) {
                amount = m
                amountAt = i
                break
            }
        }
        val amt = amount ?: return Result.NoAmount
        val at = amountAt ?: return Result.NoAmount

        val nameWords = words.filterIndexed { i, w -> i != at && w !in verbs && w !in fillers }
        val name = nameWords.joinToString(" ")
        if (name.isEmpty()) return Result.NoPayee("")

        val hits = payees.filter { p ->
            val n = p.name.lowercase()
            n == name || n.contains(name) || nameWords.all { n.contains(it) } || name.contains(n)
        }
        val exact = hits.filter { it.name.lowercase() == name }
        if (exact.size == 1) return Result.Pay(exact[0], amt)
        if (hits.size == 1) return Result.Pay(hits[0], amt)
        if (hits.isEmpty()) return Result.NoPayee(name)
        return Result.Ambiguous(hits, amt)
    }
}
