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

    fun ageing(): Ageing = get("/v1/collections/ageing")
    fun reminderDrafts(): List<ReminderDraft> = get<ReminderDraftPage>("/v1/collections/drafts").data

    fun approveReminderDraft(id: String, message: String? = null) {
        val body = buildJsonObject { message?.let { put("message", it) } }
        post<Unit>("/v1/collections/drafts/${enc(id)}/approve", body)
    }

    fun rejectReminderDraft(id: String) {
        post<Unit>("/v1/collections/drafts/${enc(id)}/reject", buildJsonObject { })
    }

    fun emailInvoice(id: String) {
        post<Unit>("/v1/invoices/${enc(id)}/send", buildJsonObject { })
    }

    fun pendingApprovals(): List<PendingApproval> {
        val refunds = get<RefundPage>("/v1/refunds?status=pending_approval").data
            .filter { it.status == "pending_approval" }
            .map {
                PendingApproval(
                    PendingApproval.Kind.Refund, it.id, it.amountMinor, it.currency,
                    it.reason.orEmpty(), it.requestedBy,
                )
            }
        val payouts = get<PayoutPage>("/v1/payouts?status=pending_approval").data
            .filter { it.status == "pending_approval" }
            .map {
                PendingApproval(
                    PendingApproval.Kind.Payout, it.id, it.amountMinor, it.currency,
                    it.purpose.orEmpty(), it.requestedBy,
                )
            }
        return refunds + payouts
    }

    fun decideApproval(a: PendingApproval, approve: Boolean) {
        val base = if (a.kind == PendingApproval.Kind.Refund) "/v1/refunds" else "/v1/payouts"
        val action = if (approve) "approve" else "reject"
        post<Unit>("$base/${enc(a.rawId)}/$action", buildJsonObject { })
    }

    fun cashflow(weeks: Int = 4): Cashflow {
        val w = weeks.coerceIn(1, 12)
        return get("/v1/insights/cashflow?weeks=$w")
    }

    fun settlements(): List<SettlementInfo> = get<SettlementPage>("/v1/settlements").data

    fun gstGstr1(period: String): GstSummary = get("/v1/gst/gstr1?period=${enc(period)}")

    fun listMandates(): List<MandateInfo> = get<MandatePage>("/v1/mandates").data

    fun pauseMandate(id: String) = post<Unit>("/v1/mandates/${enc(id)}/pause", buildJsonObject { })
    fun resumeMandate(id: String) = post<Unit>("/v1/mandates/${enc(id)}/resume", buildJsonObject { })
    fun revokeMandate(id: String) = post<Unit>("/v1/mandates/${enc(id)}/revoke", buildJsonObject { })

    fun simMandateDecide(mandateId: String, approve: Boolean = true): SimPayResult {
        val body = buildJsonObject {
            put("mandate_id", mandateId)
            put("approve", approve)
        }
        return post("/v1/sim/mandate", body)
    }

    fun simCircleCreate(primary: String, delegate: String, perPayment: Money, total: Money): String {
        val body = buildJsonObject {
            put("primary", primary)
            put("delegate", delegate)
            put("max_per_payment_minor", perPayment.minor)
            put("total_limit_minor", total.minor)
        }
        return post<CircleCreated>("/v1/sim/circle", body).circleId
    }

    fun simCircleView(id: String): CircleInfo {
        require(MandatePayload.isSafeId(id)) { "Not a circle id" }
        return get("/v1/sim/circle/${enc(id)}")
    }

    fun simCircleRevoke(id: String) {
        post<Unit>("/v1/sim/circle/revoke", buildJsonObject { put("circle_id", id) })
    }

    fun simCirclePay(circleId: String, upiUri: String): CirclePayResult {
        val body = buildJsonObject {
            put("circle_id", circleId)
            put("upi_uri", upiUri)
        }
        return post("/v1/sim/circle/pay", body)
    }

    fun branches(): List<BranchInfo> = get<BranchPage>("/v1/branches").data

    fun createBranch(name: String): BranchInfo =
        post("/v1/branches", buildJsonObject { put("name", name.trim()) })

    fun addCash(branchId: String, kind: String, amount: Money, note: String? = null) {
        val body = buildJsonObject {
            put("branch_id", branchId)
            put("kind", kind)
            put("amount_minor", amount.minor)
            note?.takeIf { it.isNotBlank() }?.let { put("note", it) }
        }
        post<Unit>("/v1/cash_entries", body, idem = true)
    }

    fun closeDay(branchId: String, day: String, counted: Money, note: String? = null): ClosingInfo {
        val body = buildJsonObject {
            put("branch_id", branchId)
            put("day", day)
            put("counted_cash_minor", counted.minor)
            note?.takeIf { it.isNotBlank() }?.let { put("note", it) }
        }
        return post("/v1/closings", body)
    }

    fun openDisputes(): List<DisputeRow> =
        get<DisputePage>("/v1/disputes?status=open").data.filter { it.status == "open" || it.status == "needs_response" }

    fun openReconCases(): List<ReconCase> =
        get<ReconCasePage>("/v1/recon/cases?status=open").data.filter { it.status == "open" }

    fun resolveReconCase(id: String, note: String) {
        post<Unit>(
            "/v1/recon/cases/${enc(id)}/resolve",
            buildJsonObject { put("note", note) },
        )
    }

    /** Reaches the server, then (when a key is set) makes one small keyed call. */
    fun checkConnection(needsKey: Boolean = false): ConnectionVerdict {
        val address = ConnectionCheck.parse(baseUrl)
        val short = http.newBuilder()
            .connectTimeout(6, TimeUnit.SECONDS)
            .readTimeout(6, TimeUnit.SECONDS)
            .build()
        val health = probe(short, join(baseUrl, "/healthz"), withAuth = false)
        var authProbe: Probe? = null
        if (health is Probe.Status && health.code == 200 && !apiKey.isNullOrBlank()) {
            authProbe = probe(short, join(baseUrl, "/v1/payment_intents?limit=1"), withAuth = true)
        }
        return ConnectionCheck.classify(address, health, authProbe, needsKey)
    }

    private fun probe(client: OkHttpClient, url: String, withAuth: Boolean): Probe {
        return try {
            val b = Request.Builder().url(url).get()
            if (withAuth) apiKey?.takeIf { it.isNotBlank() }?.let { b.header("Authorization", "Bearer $it") }
            client.newCall(b.build()).execute().use { Probe.Status(it.code) }
        } catch (_: java.net.SocketTimeoutException) {
            Probe.Failure(timedOut = true)
        } catch (_: Exception) {
            Probe.Failure(timedOut = false)
        }
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
            if (raw.isBlank() || raw == "null" || T::class == Unit::class) {
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

    /** Set spendable balance without a history row (jar moves). */
    fun setBalanceMinor(minor: Long) {
        require(minor >= 0) { "balance cannot be negative" }
        balanceMinor = minor
    }
}
