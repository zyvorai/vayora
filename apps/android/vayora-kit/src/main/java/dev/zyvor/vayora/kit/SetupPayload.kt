package dev.zyvor.vayora.kit

import java.net.URI
import java.net.URLDecoder

/**
 * What a setup QR carries: server address + test API key.
 * Format: `vayora://setup?server=<url>&key=<zk_test_…>`. Live keys are refused.
 */
data class SetupPayload(val server: String, val apiKey: String) {
    sealed class Failure : Exception() {
        data object NotSetup : Failure()
        data object BadServer : Failure()
        data object MissingKey : Failure()
        data object LiveKeyRefused : Failure()
    }

    companion object {
        fun parse(text: String): SetupPayload {
            val t = text.trim()
            val uri = try {
                URI(t)
            } catch (_: Exception) {
                throw Failure.NotSetup
            }
            if (uri.scheme?.lowercase() != "vayora" || uri.host?.lowercase() != "setup") throw Failure.NotSetup
            val q = linkedMapOf<String, String>()
            for (item in (uri.rawQuery ?: "").split('&')) {
                if (item.isEmpty()) continue
                val eq = item.indexOf('=')
                val k = if (eq < 0) item else item.substring(0, eq)
                val v = if (eq < 0) "" else URLDecoder.decode(item.substring(eq + 1), Charsets.UTF_8)
                if (q.containsKey(k)) throw Failure.BadServer
                q[k] = v
            }
            val server = q["server"]?.takeIf { it.isNotEmpty() } ?: throw Failure.BadServer
            val parsed = ConnectionCheck.parse(server) ?: throw Failure.BadServer
            val key = q["key"]?.takeIf { it.isNotEmpty() } ?: throw Failure.MissingKey
            if (!key.startsWith("zk_test_")) throw Failure.LiveKeyRefused
            return SetupPayload(parsed.toString().trimEnd('/'), key)
        }
    }
}
