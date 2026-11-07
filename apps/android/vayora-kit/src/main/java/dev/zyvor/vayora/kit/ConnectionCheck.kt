package dev.zyvor.vayora.kit

import java.net.URI

/** What a "Test connection" press found, in words a person can act on. Mirrors iOS `ConnectionVerdict`. */
enum class ConnectionVerdict {
    Ok, BadAddress, ThisPhone, CannotReach, TimedOut, NotVayora, WrongKey, NoKey, ServerError;

    val isOk: Boolean get() = this == Ok

    val title: String
        get() = when (this) {
            Ok -> "Connected"
            BadAddress -> "That address doesn't look right"
            ThisPhone -> "That address is this phone"
            CannotReach -> "Can't reach the server"
            TimedOut -> "The server didn't answer"
            NotVayora -> "Something answered, but not a Vayora server"
            WrongKey -> "The server rejected the key"
            NoKey -> "The server is reachable, but there is no key"
            ServerError -> "The server had a problem"
        }

    fun advice(status: Int? = null): String = when (this) {
        Ok -> "The server answered and the key was accepted. Test money only."
        BadAddress -> "Use the form http://192.168.1.13:18080, with http:// or https:// and the port. On an emulator, http://10.0.2.2:8080 reaches the host machine."
        ThisPhone -> "127.0.0.1 and localhost mean this phone. On an emulator use http://10.0.2.2:8080; on a device use the computer's Wi‑Fi address."
        CannotReach -> "Check the phone is on the same Wi‑Fi as the computer, and that the server is running."
        TimedOut -> "The address is reachable in form but nothing replied. Check the server is running and the port is right."
        NotVayora -> "The address replied${status?.let { " with status $it" } ?: ""} but not like a Vayora server (/healthz). Check the port."
        WrongKey -> "Scan the setup QR again, or paste a test key that starts with zk_test_."
        NoKey -> "Scan the setup QR, or paste a test key that starts with zk_test_ into the API key box."
        ServerError -> "The server replied${status?.let { " with status $it" } ?: ""}. Look at its log."
    }
}

sealed class Probe {
    data class Status(val code: Int) : Probe()
    data class Failure(val timedOut: Boolean) : Probe()
}

object ConnectionCheck {
    fun parse(text: String): URI? {
        val t = text.trim()
        return try {
            val u = URI(t)
            val scheme = u.scheme?.lowercase()
            if ((scheme == "http" || scheme == "https") && !u.host.isNullOrBlank()) u else null
        } catch (_: Exception) {
            null
        }
    }

    fun isThisPhone(uri: URI): Boolean {
        val h = uri.host?.lowercase() ?: return false
        return h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]"
    }

    fun classify(address: URI?, health: Probe, auth: Probe?, needsKey: Boolean = false): ConnectionVerdict {
        if (address == null) return ConnectionVerdict.BadAddress
        if (isThisPhone(address)) return ConnectionVerdict.ThisPhone
        when (health) {
            is Probe.Failure -> return if (health.timedOut) ConnectionVerdict.TimedOut else ConnectionVerdict.CannotReach
            is Probe.Status -> if (health.code != 200) return ConnectionVerdict.NotVayora
        }
        if (auth == null) return if (needsKey) ConnectionVerdict.NoKey else ConnectionVerdict.Ok
        return when (auth) {
            is Probe.Failure -> if (auth.timedOut) ConnectionVerdict.TimedOut else ConnectionVerdict.CannotReach
            is Probe.Status -> when {
                auth.code in 200..299 -> ConnectionVerdict.Ok
                auth.code == 401 || auth.code == 403 -> ConnectionVerdict.WrongKey
                auth.code >= 500 -> ConnectionVerdict.ServerError
                else -> ConnectionVerdict.NotVayora
            }
        }
    }
}
