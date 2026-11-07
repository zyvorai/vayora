package dev.zyvor.vayora.kit

import kotlinx.serialization.Serializable

@Serializable
data class Payee(
    val vpa: String,
    val name: String,
    val favourite: Boolean = false,
)

object PayeeInput {
    const val PHONE_DOMAIN = "zyvor-test"

    fun vpa(from: String): String? {
        val t = from.trim()
        if (t.contains('@')) return if (UPIPayload.isValidVPA(t)) t.lowercase() else null
        var d = t.filter { it !in " -()" }
        when {
            d.startsWith("+91") -> d = d.drop(3)
            d.startsWith("0") -> d = d.drop(1)
        }
        if (d.length != 10 || !d.all { it.isDigit() } || d.first() !in "6789") return null
        return "$d@$PHONE_DOMAIN"
    }
}

class PayeeBook(initial: List<Payee> = emptyList()) {
    private val _all = initial.toMutableList()
    val all: List<Payee> get() = _all.toList()

    val sorted: List<Payee>
        get() = _all.sortedWith(
            compareByDescending<Payee> { it.favourite }.thenBy { it.name.lowercase() },
        )

    fun save(name: String, input: String): Boolean {
        val vpa = PayeeInput.vpa(input) ?: return false
        val label = name.trim()
        val i = _all.indexOfFirst { it.vpa == vpa }
        if (i >= 0) {
            if (label.isNotEmpty()) _all[i] = _all[i].copy(name = label)
        } else {
            _all.add(Payee(vpa, if (label.isEmpty()) vpa else label))
        }
        return true
    }

    fun toggleFavourite(vpa: String) {
        val i = _all.indexOfFirst { it.vpa == vpa }
        if (i >= 0) _all[i] = _all[i].copy(favourite = !_all[i].favourite)
    }
}
