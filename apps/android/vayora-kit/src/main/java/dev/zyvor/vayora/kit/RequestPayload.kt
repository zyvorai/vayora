package dev.zyvor.vayora.kit

import java.net.URI
import java.net.URLDecoder
import java.net.URLEncoder

/** `vayora://request?pa=…&pn=…&am=…` or open amount with `open=1`. */
object RequestPayload {
    sealed class Failure : Exception() {
        data object NotRequest : Failure()
        data object MissingAmount : Failure()
        data class Invalid(val upi: UPIPayload.Failure) : Failure()
    }

    fun make(payee: String, name: String, amount: Money, note: String): String {
        fun enc(s: String) = URLEncoder.encode(s, Charsets.UTF_8).replace("+", "%20")
        return "vayora://request?pa=${enc(payee)}&pn=${enc(name)}&am=${enc(amount.decimalString)}&tn=${enc(note)}"
    }

    fun makeOpen(payee: String, name: String): String {
        fun enc(s: String) = URLEncoder.encode(s, Charsets.UTF_8).replace("+", "%20")
        return "vayora://request?pa=${enc(payee)}&pn=${enc(name)}&open=1"
    }

    fun parse(text: String): UPIPayload {
        val t = text.trim()
        val uri = try {
            URI(t)
        } catch (_: Exception) {
            throw Failure.NotRequest
        }
        if (uri.scheme?.lowercase() != "vayora" || uri.host?.lowercase() != "request") throw Failure.NotRequest
        val items = (uri.rawQuery ?: "").split('&').filter { it.isNotEmpty() }
        val openValues = items.mapNotNull { item ->
            val eq = item.indexOf('=')
            val k = if (eq < 0) item else item.substring(0, eq)
            val v = if (eq < 0) "" else URLDecoder.decode(item.substring(eq + 1), Charsets.UTF_8)
            if (k == "open") v else null
        }
        if (openValues.size > 1) throw Failure.Invalid(UPIPayload.Failure.BadAmount)
        val open = openValues == listOf("1")
        val filtered = items.filter {
            val k = it.substringBefore('=', it)
            k != "open"
        }
        val asUpi = "upi://pay?" + (filtered + "cu=INR").joinToString("&")
        return try {
            val p = UPIPayload.parse(asUpi)
            if (p.amount == null && !open) throw Failure.MissingAmount
            p
        } catch (e: UPIPayload.Failure) {
            throw Failure.Invalid(upi = e)
        } catch (e: Failure) {
            throw e
        }
    }
}
