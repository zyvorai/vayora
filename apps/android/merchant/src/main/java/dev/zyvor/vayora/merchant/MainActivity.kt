package dev.zyvor.vayora.merchant

import android.content.Context
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountBalanceWallet
import androidx.compose.material.icons.filled.Payments
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.zyvor.vayora.kit.Ageing
import dev.zyvor.vayora.kit.Cashflow
import dev.zyvor.vayora.kit.ClosingInfo
import dev.zyvor.vayora.kit.ConnectionVerdict
import dev.zyvor.vayora.kit.DisputeRow
import dev.zyvor.vayora.kit.GstSummary
import dev.zyvor.vayora.kit.MandateInfo
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.PendingApproval
import dev.zyvor.vayora.kit.ReconCase
import dev.zyvor.vayora.kit.ReminderDraft
import dev.zyvor.vayora.kit.SettlementInfo
import dev.zyvor.vayora.kit.SetupPayload
import dev.zyvor.vayora.kit.VayoraClient
import dev.zyvor.vayora.ui.AmbientBackground
import dev.zyvor.vayora.ui.AutoPayListScreen
import dev.zyvor.vayora.ui.Brand
import dev.zyvor.vayora.ui.CashflowScreen
import dev.zyvor.vayora.ui.CollectKeypadScreen
import dev.zyvor.vayora.ui.ConnectionRow
import dev.zyvor.vayora.ui.EndOfDayScreen
import dev.zyvor.vayora.ui.OwedScreen
import dev.zyvor.vayora.ui.PaymentRow
import dev.zyvor.vayora.ui.PaymentRowState
import dev.zyvor.vayora.ui.PaymentsListScreen
import dev.zyvor.vayora.ui.QrHeroScreen
import dev.zyvor.vayora.ui.ScannerScreen
import dev.zyvor.vayora.ui.TestChip
import dev.zyvor.vayora.ui.Vui
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.time.LocalDate
import java.time.YearMonth
import java.time.ZoneId

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { MerchantApp() }
    }
}

private sealed interface Route {
    data object Collect : Route
    data object Payments : Route
    data object Owed : Route
    data object Settings : Route
    data object Cashflow : Route
    data object AutoPay : Route
    data object SetupScan : Route
    data object EndOfDay : Route
    data class Qr(val amount: Money, val note: String, val intentId: String?, val upi: String?, val paid: Boolean) : Route
}

@Composable
private fun MerchantApp() {
    val ctx = LocalContext.current
    val prefs = remember { ctx.getSharedPreferences("vayora_merchant", Context.MODE_PRIVATE) }
    var tab by remember { mutableIntStateOf(0) }
    var route by remember { mutableStateOf<Route>(Route.Collect) }
    var amountText by remember { mutableStateOf("") }
    var note by remember { mutableStateOf("") }
    var rows by remember { mutableStateOf<List<PaymentRow>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var ageing by remember { mutableStateOf<Ageing?>(null) }
    var drafts by remember { mutableStateOf<List<ReminderDraft>>(emptyList()) }
    var approvals by remember { mutableStateOf<List<PendingApproval>>(emptyList()) }
    var disputes by remember { mutableStateOf<List<DisputeRow>>(emptyList()) }
    var cases by remember { mutableStateOf<List<ReconCase>>(emptyList()) }
    var owedMsg by remember { mutableStateOf<String?>(null) }
    var cashflow by remember { mutableStateOf<Cashflow?>(null) }
    var settlements by remember { mutableStateOf<List<SettlementInfo>>(emptyList()) }
    var gst by remember { mutableStateOf<GstSummary?>(null) }
    var gstPeriod by remember {
        mutableStateOf(YearMonth.now(ZoneId.of("Asia/Kolkata")).minusMonths(1).toString())
    }
    var mandates by remember { mutableStateOf<List<MandateInfo>>(emptyList()) }
    var autoMsg by remember { mutableStateOf<String?>(null) }
    var verdict by remember { mutableStateOf<ConnectionVerdict?>(null) }
    var connBusy by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var branches by remember { mutableStateOf<List<Pair<String, String>>>(emptyList()) }
    var branchId by remember { mutableStateOf<String?>(null) }
    var closing by remember { mutableStateOf<ClosingInfo?>(null) }
    var eodMsg by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    fun server() = prefs.getString("server", "http://10.0.2.2:8080") ?: "http://10.0.2.2:8080"
    fun key() = prefs.getString("key", "") ?: ""
    fun demo() = prefs.getBoolean("demo", true)
    fun client() = VayoraClient(server(), key().ifBlank { null })
    fun canContinue(): Boolean = Money.parseDecimal(amountText) != null

    fun onDigit(d: Int) {
        if (amountText.contains('.') && amountText.substringAfter('.').length >= 2) return
        if (amountText == "0") amountText = d.toString() else amountText += d.toString()
    }
    fun onPoint() {
        if (!amountText.contains('.')) amountText = if (amountText.isEmpty()) "0." else "$amountText."
    }
    fun onDelete() {
        if (amountText.isNotEmpty()) amountText = amountText.dropLast(1)
    }

    fun loadPayments() {
        if (demo()) return
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { client().listPayments() } }.onSuccess { page ->
                rows = page.data.map {
                    PaymentRow(
                        id = it.id,
                        amount = it.money,
                        whenLabel = it.createdAt?.take(16)?.replace('T', ' ') ?: it.id.take(12),
                        state = when (it.status) {
                            "succeeded", "captured" -> PaymentRowState.Succeeded
                            else -> PaymentRowState.Processing
                        },
                    )
                }
            }
        }
    }

    fun loadOwed() {
        if (demo()) {
            owedMsg = "Turn Demo mode off and set a test key to load Owed."
            return
        }
        busy = true
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    ageing = client().ageing()
                    drafts = runCatching { client().reminderDrafts() }.getOrDefault(emptyList())
                    approvals = runCatching { client().pendingApprovals() }.getOrDefault(emptyList())
                    disputes = runCatching { client().openDisputes() }.getOrDefault(emptyList())
                    cases = runCatching { client().openReconCases() }.getOrDefault(emptyList())
                }
                owedMsg = null
            } catch (e: Exception) {
                owedMsg = e.message
            } finally {
                busy = false
            }
        }
    }

    fun loadCashflow() {
        if (demo()) return
        scope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    cashflow = client().cashflow()
                    settlements = client().settlements()
                    gst = runCatching { client().gstGstr1(gstPeriod) }.getOrNull()
                }
            }.onFailure { error = it.message }
        }
    }

    fun loadMandates() {
        if (demo()) {
            autoMsg = "Turn Demo mode off to list mandates."
            return
        }
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { mandates = client().listMandates() } }
                .onSuccess { autoMsg = null }
                .onFailure { autoMsg = it.message }
        }
    }

    fun loadBranches() {
        if (demo()) {
            eodMsg = "Turn Demo mode off and set a test key for end of day."
            return
        }
        scope.launch {
            runCatching {
                withContext(Dispatchers.IO) { client().branches() }
            }.onSuccess { list ->
                branches = list.map { it.id to it.name }
                if (branchId == null) branchId = list.firstOrNull()?.id
                eodMsg = null
            }.onFailure { eodMsg = it.message }
        }
    }

    val showBar = route is Route.Collect || route is Route.Payments || route is Route.Owed || route is Route.Settings

    Scaffold(
        containerColor = Vui.Ink,
        bottomBar = {
            if (showBar) {
                NavigationBar(containerColor = Vui.InkRaised) {
                    listOf(
                        Triple(0, "Collect", Icons.Filled.QrCode),
                        Triple(1, "Payments", Icons.Filled.Payments),
                        Triple(2, "Owed", Icons.Filled.AccountBalanceWallet),
                        Triple(3, "Settings", Icons.Filled.Settings),
                    ).forEach { (i, label, icon) ->
                        NavigationBarItem(
                            selected = tab == i,
                            onClick = {
                                tab = i
                                route = when (i) {
                                    1 -> {
                                        loadPayments()
                                        Route.Payments
                                    }
                                    2 -> {
                                        loadOwed()
                                        Route.Owed
                                    }
                                    3 -> Route.Settings
                                    else -> Route.Collect
                                }
                            },
                            icon = { androidx.compose.material3.Icon(icon, label) },
                            label = { Text(label) },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = Vui.Emerald,
                                selectedTextColor = Vui.Emerald,
                                unselectedIconColor = Vui.TextTertiary,
                                unselectedTextColor = Vui.TextTertiary,
                                indicatorColor = Vui.Emerald.copy(alpha = 0.2f),
                            ),
                        )
                    }
                }
            }
        },
    ) { pad ->
        Box(Modifier.fillMaxSize().padding(pad)) {
            when (val r = route) {
                Route.Collect -> CollectKeypadScreen(
                    text = amountText,
                    note = note,
                    canContinue = canContinue(),
                    onDigit = ::onDigit,
                    onPoint = ::onPoint,
                    onDelete = ::onDelete,
                    onChip = { note = if (note == it) "" else it },
                    onShowQr = {
                        val money = Money.parseDecimal(amountText) ?: return@CollectKeypadScreen
                        if (demo()) {
                            val fake = "upi://pay?pa=sandbox@zyvor-sim&pn=Vayora%20Sandbox&am=${money.decimalString}&cu=INR&tn=demo_${System.currentTimeMillis()}"
                            route = Route.Qr(money, note, null, fake, false)
                            scope.launch {
                                delay(3500)
                                if (route is Route.Qr) {
                                    route = Route.Qr(money, note, null, fake, true)
                                    rows = listOf(
                                        PaymentRow("demo_${System.currentTimeMillis()}", money, "Just now", note, PaymentRowState.Succeeded),
                                    ) + rows
                                }
                            }
                        } else {
                            scope.launch {
                                try {
                                    error = null
                                    val (intent, attempt) = withContext(Dispatchers.IO) {
                                        val i = client().createIntent(money.minor, description = note.ifBlank { null })
                                        i to client().createAttempt(i.id)
                                    }
                                    route = Route.Qr(money, note, intent.id, attempt.upiURI, false)
                                } catch (e: Exception) {
                                    error = e.message
                                }
                            }
                        }
                    },
                )
                Route.Payments -> Column(Modifier.fillMaxSize()) {
                    Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                        TextButton(onClick = {
                            loadCashflow()
                            route = Route.Cashflow
                        }) { Text("Cashflow", color = Vui.Emerald) }
                        TextButton(onClick = {
                            loadMandates()
                            route = Route.AutoPay
                        }) { Text("AutoPay", color = Vui.Emerald) }
                    }
                    PaymentsListScreen(rows)
                }
                Route.Owed -> OwedScreen(
                    ageing = ageing,
                    drafts = drafts,
                    approvals = approvals,
                    disputes = disputes,
                    cases = cases,
                    busy = busy,
                    message = owedMsg,
                    onRefresh = ::loadOwed,
                    onApproveDraft = { d ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().approveReminderDraft(d.id) } }
                                .onSuccess { owedMsg = "Reminder approved."; loadOwed() }
                                .onFailure { owedMsg = it.message }
                        }
                    },
                    onRejectDraft = { d ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().rejectReminderDraft(d.id) } }
                                .onSuccess { owedMsg = "Reminder rejected."; loadOwed() }
                                .onFailure { owedMsg = it.message }
                        }
                    },
                    onEmailInvoice = { id ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().emailInvoice(id) } }
                                .onSuccess { owedMsg = "Invoice emailed." }
                                .onFailure { owedMsg = it.message }
                        }
                    },
                    onDecide = { a, ok ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().decideApproval(a, ok) } }
                                .onSuccess {
                                    owedMsg = if (ok) "${a.title} approved." else "${a.title} rejected."
                                    loadOwed()
                                }
                                .onFailure { owedMsg = it.message }
                        }
                    },
                    onResolveCase = { c ->
                        scope.launch {
                            runCatching {
                                withContext(Dispatchers.IO) { client().resolveReconCase(c.id, "Checked on phone") }
                            }.onSuccess { owedMsg = "Case closed."; loadOwed() }
                                .onFailure { owedMsg = it.message }
                        }
                    },
                )
                Route.Cashflow -> CashflowScreen(
                    cashflow = cashflow,
                    settlements = settlements,
                    gst = gst,
                    gstPeriod = gstPeriod,
                    onBack = { route = Route.Payments; tab = 1 },
                    onPrevMonth = {
                        gstPeriod = YearMonth.parse(gstPeriod).minusMonths(1).toString()
                        loadCashflow()
                    },
                    onNextMonth = {
                        val next = YearMonth.parse(gstPeriod).plusMonths(1)
                        val cap = YearMonth.now(ZoneId.of("Asia/Kolkata"))
                        if (!next.isAfter(cap)) {
                            gstPeriod = next.toString()
                            loadCashflow()
                        }
                    },
                )
                Route.AutoPay -> AutoPayListScreen(
                    mandates = mandates,
                    message = autoMsg,
                    onRefresh = ::loadMandates,
                    onPause = { m ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().pauseMandate(m.id) } }
                                .onSuccess { autoMsg = "Paused."; loadMandates() }
                                .onFailure { autoMsg = it.message }
                        }
                    },
                    onResume = { m ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().resumeMandate(m.id) } }
                                .onSuccess { autoMsg = "Resumed."; loadMandates() }
                                .onFailure { autoMsg = it.message }
                        }
                    },
                    onRevoke = { m ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { client().revokeMandate(m.id) } }
                                .onSuccess { autoMsg = "Revoked."; loadMandates() }
                                .onFailure { autoMsg = it.message }
                        }
                    },
                    onBack = { route = Route.Payments; tab = 1 },
                )
                Route.Settings -> SettingsScreen(
                    server = server(),
                    key = key(),
                    demo = demo(),
                    verdict = verdict,
                    connBusy = connBusy,
                    onTest = {
                        connBusy = true
                        scope.launch {
                            verdict = withContext(Dispatchers.IO) { client().checkConnection(needsKey = true) }
                            connBusy = false
                        }
                    },
                    onScanSetup = { route = Route.SetupScan },
                    onEndOfDay = {
                        loadBranches()
                        closing = null
                        route = Route.EndOfDay
                    },
                    onSave = { s, k, d ->
                        prefs.edit().putString("server", s).putString("key", k).putBoolean("demo", d).apply()
                    },
                )
                Route.EndOfDay -> EndOfDayScreen(
                    branches = branches,
                    selectedBranchId = branchId,
                    closing = closing,
                    message = eodMsg,
                    busy = busy,
                    onSelectBranch = { branchId = it },
                    onCreateBranch = { name ->
                        scope.launch {
                            try {
                                val b = withContext(Dispatchers.IO) { client().createBranch(name) }
                                branches = branches + (b.id to b.name)
                                branchId = b.id
                                eodMsg = "Branch created."
                            } catch (e: Exception) {
                                eodMsg = e.message
                            }
                        }
                    },
                    onAddCash = { kind, amount ->
                        val id = branchId ?: return@EndOfDayScreen
                        scope.launch {
                            try {
                                withContext(Dispatchers.IO) { client().addCash(id, kind, amount) }
                                eodMsg = "Cash $kind recorded."
                            } catch (e: Exception) {
                                eodMsg = e.message
                            }
                        }
                    },
                    onClose = { counted ->
                        val id = branchId ?: return@EndOfDayScreen
                        busy = true
                        scope.launch {
                            try {
                                val day = LocalDate.now(ZoneId.of("Asia/Kolkata")).toString()
                                closing = withContext(Dispatchers.IO) { client().closeDay(id, day, counted) }
                                eodMsg = "Closed. A second person signs off on the console."
                            } catch (e: Exception) {
                                eodMsg = e.message
                            } finally {
                                busy = false
                            }
                        }
                    },
                    onBack = { route = Route.Settings; tab = 3 },
                )
                Route.SetupScan -> ScannerScreen(
                    brand = Brand.Merchant,
                    hint = "Scan the setup QR",
                    onCode = { raw ->
                        try {
                            val p = SetupPayload.parse(raw)
                            prefs.edit().putString("server", p.server).putString("key", p.apiKey).putBoolean("demo", false).apply()
                            route = Route.Settings
                            tab = 3
                        } catch (e: SetupPayload.Failure) {
                            error = when (e) {
                                SetupPayload.Failure.LiveKeyRefused -> "Live keys are refused. Use a zk_test_ key."
                                SetupPayload.Failure.NotSetup -> "Not a setup QR."
                                SetupPayload.Failure.BadServer -> "That server address looks wrong."
                                SetupPayload.Failure.MissingKey -> "The QR has no API key."
                            }
                            route = Route.Settings
                            tab = 3
                        }
                    },
                    onCancel = { route = Route.Settings; tab = 3 },
                )
                is Route.Qr -> {
                    if (!r.paid && r.intentId != null && !demo()) {
                        LaunchedEffect(r.intentId) {
                            while (isActive) {
                                delay(1500)
                                runCatching {
                                    withContext(Dispatchers.IO) { client().getIntent(r.intentId) }
                                }.onSuccess { intent ->
                                    if (intent.status == "succeeded" || intent.status == "captured") {
                                        route = r.copy(paid = true)
                                        loadPayments()
                                    }
                                }
                            }
                        }
                    }
                    QrHeroScreen(
                        amount = r.amount,
                        upiUri = r.upi,
                        note = r.note,
                        paid = r.paid,
                        onCancel = {
                            amountText = ""
                            note = ""
                            route = Route.Collect
                            tab = 0
                        },
                    )
                }
            }
            error?.let {
                Text(it, color = Vui.Coral, modifier = Modifier.padding(16.dp).align(Alignment.BottomCenter))
            }
        }
    }
}

@Composable
private fun SettingsScreen(
    server: String,
    key: String,
    demo: Boolean,
    verdict: ConnectionVerdict?,
    connBusy: Boolean,
    onTest: () -> Unit,
    onScanSetup: () -> Unit,
    onEndOfDay: () -> Unit,
    onSave: (String, String, Boolean) -> Unit,
) {
    var s by remember { mutableStateOf(server) }
    var k by remember { mutableStateOf(key) }
    var d by remember { mutableStateOf(demo) }
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(Modifier.fillMaxSize().padding(20.dp)) {
            Text("Settings", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(8.dp))
            TestChip()
            Spacer(Modifier.height(16.dp))
            val fieldColors = OutlinedTextFieldDefaults.colors(
                focusedTextColor = Vui.TextPrimary,
                unfocusedTextColor = Vui.TextPrimary,
                focusedBorderColor = Vui.Emerald,
                unfocusedBorderColor = Vui.Hairline,
            )
            OutlinedTextField(
                value = s,
                onValueChange = { s = it },
                label = { Text("Test server", color = Vui.TextSecondary) },
                modifier = Modifier.fillMaxWidth(),
                colors = fieldColors,
            )
            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = k,
                onValueChange = { k = it },
                label = { Text("API key (zk_test_…)", color = Vui.TextSecondary) },
                modifier = Modifier.fillMaxWidth(),
                colors = fieldColors,
            )
            TextButton(onClick = onScanSetup) { Text("Scan setup QR", color = Vui.Emerald) }
            TextButton(onClick = onEndOfDay) { Text("End of day", color = Vui.Emerald) }
            ConnectionRow(verdict, connBusy, onTest)
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = d, onCheckedChange = { d = it })
                Text(" Demo mode (no server)", color = Vui.TextPrimary)
            }
            Spacer(Modifier.weight(1f))
            Button(onClick = { onSave(s, k, d) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
                Text("Save")
            }
        }
    }
}
