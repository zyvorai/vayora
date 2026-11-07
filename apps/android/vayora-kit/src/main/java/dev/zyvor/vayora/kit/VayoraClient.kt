package dev.zyvor.vayora.kit

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.UUID
import java.util.concurrent.TimeUnit

/**
 * Talks to a Vayora API. Transport is injectable so tests never touch the network.
 * Mirrors `VayoraClient` on iOS for the merchant / payer flows we need on Android first.
 */
class VayoraClient(
    var baseUrl: String,
    var apiKey: String? = null,
    private val http: OkHttpClient = defaultClient(),
) {
    private val jsonMedia = "application/json; charset=utf-8".toMediaType()

    fun healthz(): Boolean {
        val req = Request.Builder().url(join(baseUrl, "/healthz")).get().build()
        http.newCall(req).execute().use { return it.isSuccessful }
    }

    fun createIntent(amountMinor: Long, currency: String = "INR", description: String? = null): PaymentIntent {
        val body = buildJsonObject {
            put("amount_minor", amountMinor)
            put("currency", currency)
            description?.let { put("description", it) }
        }
        return post("/v1/payment_intents", body, idem = true)
    }

    fun getIntent(id: String): PaymentIntent = get("/v1/payment_intents/${enc(id)}")

    fun createAttempt(intentId: String, method: String = "upi", upiFlow: String = "intent"): AttemptInfo {
        val body = buildJsonObject {
            put("method", method)
            put("upi_flow", upiFlow)
        }
        return post("/v1/payment_intents/${enc(intentId)}/attempts", body, idem = true)
    }

    fun listPayments(): PagePaymentIntents = get("/v1/payment_intents")

    fun refund(intentId: String, amountMinor: Long? = null, reason: String? = null): RefundInfo {
        val body = buildJsonObject {
            amountMinor?.let { put("amount_minor", it) }
            reason?.let { put("reason", it) }
        }
        return post("/v1/payment_intents/${enc(intentId)}/refunds", body, idem = true)
    }

    /** Simulated payer: approve or decline a simulator UPI payload (`ZYVOR_SIM_PAYER=1`). */
    fun simPay(upiUri: String, approve: Boolean = true): SimPayResult {
        val body = buildJsonObject {
            put("upi_uri", upiUri)
            put("approve", approve)
        }
        return post("/v1/sim/pay", body)
    }

    private inline fun <reified T> get(path: String): T =
        execute(auth(Request.Builder().url(join(baseUrl, path)).get()).build())

    private inline fun <reified T> post(path: String, body: JsonObject, idem: Boolean = false): T {
        val builder = auth(
            Request.Builder()
                .url(join(baseUrl, path))
                .post(json.encodeToString(body).toRequestBody(jsonMedia)),
        )
        if (idem) builder.header("Idempotency-Key", UUID.randomUUID().toString())
        return execute(builder.build())
    }

    private fun auth(builder: Request.Builder): Request.Builder {
        apiKey?.takeIf { it.isNotBlank() }?.let { builder.header("Authorization", "Bearer $it") }
        return builder
    }

    private inline fun <reified T> execute(req: Request): T {
        http.newCall(req).execute().use { res ->
            val raw = res.body?.string().orEmpty()
            if (!res.isSuccessful) throw parseApiError(res.code, raw)
            if (raw.isBlank() || raw == "null") {
                @Suppress("UNCHECKED_CAST")
                return Unit as T
            }
            return json.decodeFromString(raw)
        }
    }

    companion object {
        fun defaultClient(): OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .build()

        fun join(base: String, path: String): String {
            val b = base.trimEnd('/')
            return if (path.startsWith("/")) "$b$path" else "$b/$path"
        }

        private fun enc(s: String) =
            java.net.URLEncoder.encode(s, Charsets.UTF_8).replace("+", "%20")
    }
}

/** Local test wallet held on the phone (person-to-person / demo). Not a bank. */
data class LocalTxn(
    val id: String,
    val name: String,
    val payee: String,
    val amount: Money,
    val whenLabel: String,
    val ok: Boolean = true,
)

class TestWallet(initialMinor: Long = 50_000_00L) {
    var balanceMinor: Long = initialMinor
        private set
    private val _history = mutableListOf<LocalTxn>()
    val history: List<LocalTxn> get() = _history.toList()
    val balance: Money get() = Money(balanceMinor)

    fun debit(name: String, payee: String, amount: Money, whenLabel: String = "Just now"): LocalTxn {
        require(amount.minor > 0)
        require(balanceMinor >= amount.minor) { "insufficient test balance" }
        balanceMinor -= amount.minor
        val t = LocalTxn(UUID.randomUUID().toString(), name, payee, amount, whenLabel, true)
        _history.add(0, t)
        return t
    }
}
