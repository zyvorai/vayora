package dev.zyvor.vayora.kit

/** When the app should ask for biometrics or the device credential again. Mirrors iOS `LockPolicy`. */
object LockPolicy {
    fun shouldLock(
        enabled: Boolean,
        backgroundedAtMillis: Long?,
        nowMillis: Long,
        timeoutMinutes: Int,
    ): Boolean {
        if (!enabled) return false
        if (backgroundedAtMillis == null) return true
        return nowMillis - backgroundedAtMillis >= maxOf(0, timeoutMinutes) * 60_000L
    }

    /**
     * On return to foreground: no recorded background time means the app never left
     * (permission sheets), so do not lock.
     */
    fun shouldLockOnReturn(
        enabled: Boolean,
        backgroundedAtMillis: Long?,
        nowMillis: Long,
        timeoutMinutes: Int,
    ): Boolean {
        if (backgroundedAtMillis == null) return false
        return shouldLock(enabled, backgroundedAtMillis, nowMillis, timeoutMinutes)
    }
}

object ScamHints {
    private val words = listOf(
        "refund", "cashback", "reward", "prize", "lottery", "kyc", "claim",
        "receive money", "get money", "winning", "gift card", "verify",
    )

    fun check(name: String, note: String): String? {
        val t = "$name $note".lowercase()
        if (words.none { t.contains(it) }) return null
        return "Paying never brings money to you. If someone asked you to scan or pay to get a refund, prize or cashback, it is a scam."
    }
}
