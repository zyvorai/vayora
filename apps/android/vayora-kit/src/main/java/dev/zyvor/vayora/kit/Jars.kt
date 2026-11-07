package dev.zyvor.vayora.kit

import java.util.UUID

/** A goal you set test money aside for. Money moves between wallet and jar; never created or lost. */
data class Jar(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val targetMinor: Long,
    val savedMinor: Long = 0,
) {
    val fraction: Double get() = if (targetMinor > 0) minOf(1.0, savedMinor.toDouble() / targetMinor) else 0.0
    val reached: Boolean get() = savedMinor >= targetMinor
}

class JarBook {
    sealed class Failure : Exception() {
        data object BadName : Failure()
        data object BadTarget : Failure()
        data object TooMany : Failure()
        data object BadAmount : Failure()
        data object UnknownJar : Failure()
        data object WalletTooLow : Failure()
        data object JarTooLow : Failure()
    }

    companion object {
        const val MAX_JARS = 10
    }

    private val _jars = mutableListOf<Jar>()
    val jars: List<Jar> get() = _jars.toList()
    val totalSavedMinor: Long get() = _jars.sumOf { it.savedMinor }

    fun add(name: String, targetMinor: Long): Jar {
        val n = name.trim()
        if (n.isEmpty() || n.length > 30) throw Failure.BadName
        if (targetMinor <= 0 || targetMinor > 100_000_00_00L) throw Failure.BadTarget
        if (_jars.size >= MAX_JARS) throw Failure.TooMany
        val j = Jar(name = n, targetMinor = targetMinor)
        _jars.add(j)
        return j
    }

    fun deposit(id: String, amountMinor: Long, wallet: Long): Pair<Long, JarBook> {
        if (amountMinor <= 0) throw Failure.BadAmount
        val i = _jars.indexOfFirst { it.id == id }.takeIf { it >= 0 } ?: throw Failure.UnknownJar
        if (amountMinor > wallet) throw Failure.WalletTooLow
        _jars[i] = _jars[i].copy(savedMinor = _jars[i].savedMinor + amountMinor)
        return wallet - amountMinor to this
    }

    fun withdraw(id: String, amountMinor: Long, wallet: Long): Pair<Long, JarBook> {
        if (amountMinor <= 0) throw Failure.BadAmount
        val i = _jars.indexOfFirst { it.id == id }.takeIf { it >= 0 } ?: throw Failure.UnknownJar
        if (amountMinor > _jars[i].savedMinor) throw Failure.JarTooLow
        _jars[i] = _jars[i].copy(savedMinor = _jars[i].savedMinor - amountMinor)
        return wallet + amountMinor to this
    }

    fun remove(id: String, wallet: Long): Long {
        val i = _jars.indexOfFirst { it.id == id }
        if (i < 0) return wallet
        val back = _jars[i].savedMinor
        _jars.removeAt(i)
        return wallet + back
    }
}
