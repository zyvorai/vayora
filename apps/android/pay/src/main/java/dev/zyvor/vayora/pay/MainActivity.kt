package dev.zyvor.vayora.pay

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.speech.RecognizerIntent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Home
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
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
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.zyvor.vayora.kit.BudgetCheck
import dev.zyvor.vayora.kit.Budgets
import dev.zyvor.vayora.kit.CirclePayload
import dev.zyvor.vayora.kit.CircleRecord
import dev.zyvor.vayora.kit.CircleRole
import dev.zyvor.vayora.kit.ConnectionVerdict
import dev.zyvor.vayora.kit.JarBook
import dev.zyvor.vayora.kit.LockPolicy
import dev.zyvor.vayora.kit.MandatePayload
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.Payee
import dev.zyvor.vayora.kit.PayeeBook
import dev.zyvor.vayora.kit.ScamHints
import dev.zyvor.vayora.kit.SpendCategory
import dev.zyvor.vayora.kit.TestWallet
import dev.zyvor.vayora.kit.UPIPayload
import dev.zyvor.vayora.kit.VayoraClient
import dev.zyvor.vayora.kit.VoiceCommand
import dev.zyvor.vayora.kit.toSpendEntry
import dev.zyvor.vayora.ui.AmbientBackground
import dev.zyvor.vayora.ui.Brand
import dev.zyvor.vayora.ui.BudgetsScreen
import dev.zyvor.vayora.ui.CircleScreen
import dev.zyvor.vayora.ui.ConnectionRow
import dev.zyvor.vayora.ui.JarsScreen
import dev.zyvor.vayora.ui.MandateApproveScreen
import dev.zyvor.vayora.ui.PasteLinkScreen
import dev.zyvor.vayora.ui.PayHomeScreen
import dev.zyvor.vayora.ui.PayReviewScreen
import dev.zyvor.vayora.ui.ScannerScreen
import dev.zyvor.vayora.ui.TestChip
import dev.zyvor.vayora.ui.VoicePayScreen
import dev.zyvor.vayora.ui.Vui
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { PayApp() }
    }
}

private sealed interface PayRoute {
    data object Home : PayRoute
    data object History : PayRoute
    data object Settings : PayRoute
    data object Paste : PayRoute
    data object Scan : PayRoute
    data object ScanCircle : PayRoute
    data object Jars : PayRoute
    data object Budgets : PayRoute
    data object Circle : PayRoute
    data object Voice : PayRoute
    data class Review(val payload: UPIPayload, val raw: String, val transfer: Boolean = false) : PayRoute
    data class Mandate(val payload: MandatePayload, val raw: String) : PayRoute
}

@Composable
private fun PayApp() {
    val ctx = LocalContext.current
    val activity = ctx as FragmentActivity
    val prefs = remember { ctx.getSharedPreferences("vayora_pay", Context.MODE_PRIVATE) }
    val wallet = remember { TestWallet(prefs.getLong("balance", 50_000_00L)) }
    val jars = remember { JarBook() }
    val payees = remember {
        PayeeBook(
            listOf(
                Payee("asha@zyvor-test", "Asha"),
                Payee("ravi@zyvor-test", "Ravi Kumar"),
                Payee("ravi2@zyvor-test", "Ravi Shah"),
            ),
        )
    }
    var budgets by remember { mutableStateOf(loadBudgets(prefs)) }
    var circles by remember { mutableStateOf(loadCircles(prefs)) }
    var circleQr by remember { mutableStateOf<String?>(null) }
    var circleMsg by remember { mutableStateOf<String?>(null) }
    var selectedCircleId by remember { mutableStateOf<String?>(null) }
    var voiceText by remember { mutableStateOf("") }
    var voiceMsg by remember { mutableStateOf<String?>(null) }
    var hearing by remember { mutableStateOf(false) }
    var tab by remember { mutableIntStateOf(0) }
    var route by remember { mutableStateOf<PayRoute>(PayRoute.Home) }
    var paste by remember { mutableStateOf("") }
    var pasteError by remember { mutableStateOf<String?>(null) }
    var pin by remember { mutableStateOf("") }
    var payError by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var tick by remember { mutableIntStateOf(0) }
    var verdict by remember { mutableStateOf<ConnectionVerdict?>(null) }
    var connBusy by remember { mutableStateOf(false) }
    var lockEnabled by remember { mutableStateOf(prefs.getBoolean("lock", false)) }
    var locked by remember { mutableStateOf(lockEnabled) }
    var backgroundedAt by remember { mutableLongStateOf(0L) }
    val scope = rememberCoroutineScope()
    val lifecycle = LocalLifecycleOwner.current

    fun server() = prefs.getString("server", "http://10.0.2.2:8080") ?: "http://10.0.2.2:8080"
    fun demo() = prefs.getBoolean("demo", true)
    fun refresh() {
        prefs.edit().putLong("balance", wallet.balanceMinor).apply()
        tick++
    }
    fun saveBudgets(b: Budgets) {
        budgets = b
        prefs.edit().putString("budgets", b.limits.entries.joinToString(";") { "${it.key}=${it.value}" }).apply()
    }
    fun saveCircles(list: List<CircleRecord>) {
        circles = list
        prefs.edit().putString(
            "circles",
            list.joinToString("|") {
                listOf(it.id, it.role.name, it.other, it.perPaymentMinor, it.totalMinor, it.status, it.remainingMinor).joinToString(",")
            },
        ).apply()
    }
    fun spendEntries() = wallet.history.map {
        it.toSpendEntry(SpendCategory.guess(it.name, "", false))
    }

    val speech = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        hearing = false
        val spoken = result.data?.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS)?.firstOrNull().orEmpty()
        voiceText = spoken
        when (val parsed = VoiceCommand.parse(spoken, payees.all)) {
            is VoiceCommand.Result.Pay -> {
                val raw = "upi://pay?pa=${parsed.payee.vpa}&pn=${parsed.payee.name}&am=${parsed.amount.decimalString}&cu=INR"
                route = PayRoute.Review(UPIPayload.parse(raw), raw, transfer = true)
                pin = ""
                payError = null
                voiceMsg = null
            }
            is VoiceCommand.Result.Ambiguous -> voiceMsg = "Which ${parsed.payees.joinToString { it.name }}?"
            is VoiceCommand.Result.NoPayee -> voiceMsg = if (parsed.name.isBlank()) "Say who to pay." else "No saved payee matching “${parsed.name}”."
            VoiceCommand.Result.NoAmount -> voiceMsg = "Say an amount too."
            VoiceCommand.Result.NotAPayment -> voiceMsg = "That didn’t sound like a payment."
        }
    }

    fun unlock() {
        val executor = ContextCompat.getMainExecutor(ctx)
        val prompt = BiometricPrompt(
            activity,
            executor,
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    locked = false
                }
            },
        )
        val can = BiometricManager.from(ctx).canAuthenticate(
            BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL,
        )
        if (can == BiometricManager.BIOMETRIC_SUCCESS) {
            prompt.authenticate(
                BiometricPrompt.PromptInfo.Builder()
                    .setTitle("Unlock Vayora Pay")
                    .setAllowedAuthenticators(
                        BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL,
                    )
                    .build(),
            )
        } else {
            locked = false
        }
    }

    DisposableEffect(lifecycle, lockEnabled) {
        val obs = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP -> backgroundedAt = System.currentTimeMillis()
                Lifecycle.Event.ON_START -> {
                    if (LockPolicy.shouldLockOnReturn(lockEnabled, backgroundedAt.takeIf { it > 0 }, System.currentTimeMillis(), 1)) {
                        locked = true
                    }
                }
                else -> Unit
            }
        }
        lifecycle.lifecycle.addObserver(obs)
        onDispose { lifecycle.lifecycle.removeObserver(obs) }
    }

    if (locked) {
        Box(Modifier.fillMaxSize()) {
            AmbientBackground(Brand.Pay)
            Column(Modifier.align(Alignment.Center).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Vayora Pay is locked", color = Vui.TextPrimary, fontSize = 22.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.height(16.dp))
                Button(onClick = { unlock() }) { Text("Unlock") }
            }
        }
        return
    }

    Scaffold(
        containerColor = Vui.Ink,
        bottomBar = {
            if (route is PayRoute.Home || route is PayRoute.History || route is PayRoute.Settings) {
                NavigationBar(containerColor = Vui.InkRaised) {
                    listOf(
                        Triple(0, "Home", Icons.Filled.Home),
                        Triple(1, "History", Icons.Filled.History),
                        Triple(2, "Settings", Icons.Filled.Settings),
                    ).forEach { (i, label, icon) ->
                        NavigationBarItem(
                            selected = tab == i,
                            onClick = {
                                tab = i
                                route = when (i) {
                                    1 -> PayRoute.History
                                    2 -> PayRoute.Settings
                                    else -> PayRoute.Home
                                }
                            },
                            icon = { androidx.compose.material3.Icon(icon, label) },
                            label = { Text(label) },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = Vui.Indigo,
                                selectedTextColor = Vui.Indigo,
                                unselectedIconColor = Vui.TextTertiary,
                                unselectedTextColor = Vui.TextTertiary,
                                indicatorColor = Vui.Indigo.copy(alpha = 0.2f),
                            ),
                        )
                    }
                }
            }
        },
    ) { pad ->
        Box(Modifier.fillMaxSize().padding(pad)) {
            @Suppress("UNUSED_EXPRESSION")
            tick
            when (val r = route) {
                PayRoute.Home, PayRoute.History -> PayHomeScreen(
                    balance = wallet.balance,
                    spentThisMonth = Money(wallet.history.filter { it.ok }.sumOf { it.amount.minor }),
                    recent = wallet.history,
                    onScan = { route = PayRoute.Scan },
                    onPaste = { route = PayRoute.Paste },
                    onVoice = { route = PayRoute.Voice; voiceMsg = null; voiceText = "" },
                    onSend = {
                        val p = payees.sorted.firstOrNull()
                        if (p != null) {
                            val raw = "upi://pay?pa=${p.vpa}&pn=${p.name}&am=100.00&cu=INR"
                            route = PayRoute.Review(UPIPayload.parse(raw), raw, transfer = true)
                            pin = ""; payError = null
                        }
                    },
                )
                PayRoute.Settings -> SettingsScreen(
                    server = server(),
                    demo = demo(),
                    lockEnabled = lockEnabled,
                    verdict = verdict,
                    connBusy = connBusy,
                    onTest = {
                        connBusy = true
                        scope.launch {
                            verdict = withContext(Dispatchers.IO) {
                                VayoraClient(server()).checkConnection(needsKey = false)
                            }
                            connBusy = false
                        }
                    },
                    onJars = { route = PayRoute.Jars },
                    onBudgets = { route = PayRoute.Budgets },
                    onCircle = { route = PayRoute.Circle; circleMsg = null },
                    onLockChange = {
                        lockEnabled = it
                        prefs.edit().putBoolean("lock", it).apply()
                        if (it) locked = true
                    },
                    onSave = { s, d ->
                        prefs.edit().putString("server", s).putBoolean("demo", d).apply()
                    },
                )
                PayRoute.Budgets -> BudgetsScreen(
                    budgets = budgets,
                    statuses = BudgetCheck.statuses(spendEntries(), budgets),
                    onSet = { c, m -> saveBudgets(budgets.set(c, m)); tick++ },
                    onBack = { route = PayRoute.Settings; tab = 2 },
                )
                PayRoute.Circle -> CircleScreen(
                    circles = circles,
                    message = circleMsg,
                    showQr = circleQr,
                    onCreate = { delegate, per, total ->
                        scope.launch {
                            try {
                                val id = if (demo()) {
                                    "cir_${System.currentTimeMillis().toString().takeLast(8)}"
                                } else {
                                    withContext(Dispatchers.IO) {
                                        VayoraClient(server()).simCircleCreate("Me", delegate, per, total)
                                    }
                                }
                                val rec = CircleRecord(id, CircleRole.Primary, delegate, per.minor, total.minor)
                                saveCircles(circles + rec)
                                circleQr = CirclePayload.make(id, "Me", per, total)
                                circleMsg = "Circle created. Show the QR to $delegate."
                                tick++
                            } catch (e: Exception) {
                                circleMsg = e.message
                            }
                        }
                    },
                    onRevoke = { c ->
                        scope.launch {
                            try {
                                if (!demo()) withContext(Dispatchers.IO) { VayoraClient(server()).simCircleRevoke(c.id) }
                                saveCircles(circles.map { if (it.id == c.id) it.copy(status = "revoked") else it })
                                circleMsg = "Taken back."
                                if (circleQr?.contains(c.id) == true) circleQr = null
                            } catch (e: Exception) {
                                circleMsg = e.message
                            }
                        }
                    },
                    onScan = { route = PayRoute.ScanCircle },
                    onBack = { route = PayRoute.Settings; tab = 2 },
                )
                PayRoute.Voice -> VoicePayScreen(
                    hearing = hearing,
                    transcript = voiceText,
                    message = voiceMsg,
                    onListen = {
                        hearing = true
                        voiceMsg = null
                        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
                            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                            putExtra(RecognizerIntent.EXTRA_LANGUAGE, "en-IN")
                            putExtra(RecognizerIntent.EXTRA_LANGUAGE_PREFERENCE, "hi-IN")
                            putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 3)
                            putExtra(RecognizerIntent.EXTRA_PROMPT, "Pay Asha 200 rupees")
                        }
                        runCatching { speech.launch(intent) }.onFailure {
                            hearing = false
                            voiceMsg = "Speech isn’t available on this device."
                        }
                    },
                    onBack = { route = PayRoute.Home; tab = 0 },
                )
                PayRoute.Jars -> JarsScreen(
                    jars = jars.jars,
                    wallet = wallet.balance,
                    onAdd = {
                        runCatching { jars.add("Goal ${jars.jars.size + 1}", 5_000_00) }
                        tick++
                    },
                    onDeposit = { j ->
                        runCatching {
                            val (w, _) = jars.deposit(j.id, 10_000, wallet.balanceMinor)
                            wallet.setBalanceMinor(w)
                        }
                        refresh()
                    },
                    onWithdraw = { j ->
                        runCatching {
                            val (w, _) = jars.withdraw(j.id, minOf(10_000, j.savedMinor), wallet.balanceMinor)
                            wallet.setBalanceMinor(w)
                        }
                        refresh()
                    },
                    onRemove = { j ->
                        wallet.setBalanceMinor(jars.remove(j.id, wallet.balanceMinor))
                        refresh()
                    },
                    onBack = { route = PayRoute.Settings; tab = 2 },
                )
                PayRoute.Scan, PayRoute.ScanCircle -> ScannerScreen(
                    brand = Brand.Pay,
                    hint = if (route is PayRoute.ScanCircle) "Scan a Circle invite" else "Aim at a pay, AutoPay or Circle QR",
                    onCode = { raw ->
                        when {
                            raw.lowercase().startsWith("vayora://circle") || route is PayRoute.ScanCircle -> {
                                try {
                                    val link = CirclePayload.parse(raw)
                                    scope.launch {
                                        try {
                                            val info = if (demo()) null else withContext(Dispatchers.IO) {
                                                VayoraClient(server()).simCircleView(link.id)
                                            }
                                            val rec = CircleRecord(
                                                id = link.id,
                                                role = CircleRole.Delegate,
                                                other = info?.primary ?: link.primary,
                                                perPaymentMinor = info?.maxPerPaymentMinor ?: link.perPayment.minor,
                                                totalMinor = info?.totalLimitMinor ?: link.total.minor,
                                                remainingMinor = info?.remainingMinor ?: link.total.minor,
                                                status = info?.status ?: "active",
                                            )
                                            saveCircles(circles.filterNot { it.id == rec.id } + rec)
                                            circleMsg = "Joined ${rec.other}'s circle."
                                            route = PayRoute.Circle
                                        } catch (e: Exception) {
                                            circleMsg = e.message
                                            route = PayRoute.Circle
                                        }
                                    }
                                } catch (_: CirclePayload.Failure) {
                                    pasteError = "That Circle QR could not be read."
                                    route = PayRoute.Paste
                                    paste = raw
                                }
                            }
                            raw.lowercase().startsWith("upi://mandate?") -> {
                                try {
                                    route = PayRoute.Mandate(MandatePayload.parse(raw), raw)
                                    payError = null
                                } catch (_: MandatePayload.Failure) {
                                    paste = raw
                                    pasteError = "That AutoPay QR could not be read."
                                    route = PayRoute.Paste
                                }
                            }
                            else -> {
                                try {
                                    val p = UPIPayload.parse(raw)
                                    route = PayRoute.Review(p, raw)
                                    paste = raw
                                    pin = ""
                                    selectedCircleId = null
                                    payError = ScamHints.check(p.name, p.note)
                                } catch (e: UPIPayload.Failure) {
                                    paste = raw
                                    pasteError = when (e) {
                                        UPIPayload.Failure.NotUPI -> "Not a upi://pay link."
                                        UPIPayload.Failure.MissingPayee, UPIPayload.Failure.BadPayee -> "That UPI ID looks wrong."
                                        UPIPayload.Failure.BadAmount -> "The amount is not valid."
                                        UPIPayload.Failure.UnsupportedCurrency -> "Only INR is supported."
                                    }
                                    route = PayRoute.Paste
                                }
                            }
                        }
                    },
                    onCancel = {
                        val fromCircle = route is PayRoute.ScanCircle
                        route = if (fromCircle) PayRoute.Circle else PayRoute.Home
                        if (!fromCircle) tab = 0
                    },
                )
                PayRoute.Paste -> PasteLinkScreen(
                    value = paste,
                    onValueChange = { paste = it; pasteError = null },
                    onContinue = {
                        try {
                            when {
                                paste.lowercase().startsWith("vayora://circle") -> {
                                    val link = CirclePayload.parse(paste)
                                    val rec = CircleRecord(
                                        link.id, CircleRole.Delegate, link.primary,
                                        link.perPayment.minor, link.total.minor,
                                    )
                                    saveCircles(circles.filterNot { it.id == rec.id } + rec)
                                    route = PayRoute.Circle
                                }
                                paste.lowercase().startsWith("upi://mandate?") -> {
                                    route = PayRoute.Mandate(MandatePayload.parse(paste), paste)
                                }
                                else -> {
                                    val p = UPIPayload.parse(paste)
                                    route = PayRoute.Review(p, paste)
                                    pin = ""
                                    selectedCircleId = null
                                    payError = ScamHints.check(p.name, p.note)
                                }
                            }
                        } catch (e: UPIPayload.Failure) {
                            pasteError = when (e) {
                                UPIPayload.Failure.NotUPI -> "Not a upi://pay link."
                                UPIPayload.Failure.MissingPayee, UPIPayload.Failure.BadPayee -> "That UPI ID looks wrong."
                                UPIPayload.Failure.BadAmount -> "The amount is not valid."
                                UPIPayload.Failure.UnsupportedCurrency -> "Only INR is supported."
                            }
                        } catch (_: MandatePayload.Failure) {
                            pasteError = "That AutoPay QR could not be read."
                        } catch (_: CirclePayload.Failure) {
                            pasteError = "That Circle QR could not be read."
                        }
                    },
                    onCancel = { route = PayRoute.Home; tab = 0 },
                    error = pasteError,
                )
                is PayRoute.Mandate -> MandateApproveScreen(
                    name = r.payload.name,
                    payee = r.payload.payee,
                    cap = r.payload.cap,
                    frequency = r.payload.frequency.title,
                    busy = busy,
                    error = payError,
                    onApprove = {
                        busy = true
                        scope.launch {
                            try {
                                if (!demo()) {
                                    withContext(Dispatchers.IO) {
                                        VayoraClient(server()).simMandateDecide(r.payload.mandateId, true)
                                    }
                                }
                                route = PayRoute.Home
                                tab = 0
                            } catch (e: Exception) {
                                payError = e.message
                            } finally {
                                busy = false
                            }
                        }
                    },
                    onDecline = {
                        busy = true
                        scope.launch {
                            try {
                                if (!demo()) {
                                    withContext(Dispatchers.IO) {
                                        VayoraClient(server()).simMandateDecide(r.payload.mandateId, false)
                                    }
                                }
                                route = PayRoute.Home
                                tab = 0
                            } catch (e: Exception) {
                                payError = e.message
                            } finally {
                                busy = false
                            }
                        }
                    },
                    onCancel = { route = PayRoute.Home; tab = 0 },
                )
                is PayRoute.Review -> {
                    val amount = r.payload.amount ?: Money(10_000)
                    val cat = SpendCategory.guess(r.payload.name, r.payload.note, r.transfer)
                    val warn = BudgetCheck.warning(amount.minor, cat, spendEntries(), budgets)?.warningText()
                    val cover = circles.filter { it.canCover(amount.minor) }.map { it.id to "${it.other}'s circle" }
                    PayReviewScreen(
                        name = r.payload.name,
                        payee = r.payload.payee,
                        amount = amount,
                        note = r.payload.note,
                        pin = pin,
                        onPinChange = { pin = it },
                        busy = busy,
                        error = payError,
                        budgetWarning = warn?.takeIf { it.isNotBlank() },
                        circleOptions = cover,
                        selectedCircleId = selectedCircleId,
                        onSelectCircle = { selectedCircleId = it },
                        onCancel = { route = PayRoute.Home; tab = 0 },
                        onConfirm = {
                            busy = true
                            payError = null
                            scope.launch {
                                try {
                                    val circleId = selectedCircleId
                                    if (circleId != null) {
                                        if (!demo()) {
                                            withContext(Dispatchers.IO) {
                                                VayoraClient(server()).simCirclePay(circleId, r.raw)
                                            }
                                        }
                                        saveCircles(
                                            circles.map {
                                                if (it.id == circleId) it.copy(remainingMinor = it.remainingMinor - amount.minor) else it
                                            },
                                        )
                                    } else {
                                        if (!demo() && r.payload.payee.contains("zyvor-sim")) {
                                            withContext(Dispatchers.IO) {
                                                VayoraClient(server()).simPay(r.raw)
                                            }
                                        }
                                        wallet.debit(r.payload.name.ifBlank { r.payload.payee }, r.payload.payee, amount)
                                        payees.save(r.payload.name, r.payload.payee)
                                        refresh()
                                    }
                                    route = PayRoute.Home
                                    tab = 0
                                    paste = ""
                                    selectedCircleId = null
                                } catch (e: Exception) {
                                    payError = e.message ?: "Payment failed"
                                } finally {
                                    busy = false
                                }
                            }
                        },
                    )
                }
            }
        }
    }
}

private fun loadBudgets(prefs: android.content.SharedPreferences): Budgets {
    val raw = prefs.getString("budgets", "").orEmpty()
    if (raw.isBlank()) return Budgets()
    val map = raw.split(';').mapNotNull {
        val p = it.split('=', limit = 2)
        if (p.size == 2) p[0] to (p[1].toLongOrNull() ?: return@mapNotNull null) else null
    }.toMap()
    return Budgets(map)
}

private fun loadCircles(prefs: android.content.SharedPreferences): List<CircleRecord> {
    val raw = prefs.getString("circles", "").orEmpty()
    if (raw.isBlank()) return emptyList()
    return raw.split('|').mapNotNull { row ->
        val p = row.split(',')
        if (p.size < 7) return@mapNotNull null
        CircleRecord(
            id = p[0],
            role = runCatching { CircleRole.valueOf(p[1]) }.getOrDefault(CircleRole.Primary),
            other = p[2],
            perPaymentMinor = p[3].toLongOrNull() ?: return@mapNotNull null,
            totalMinor = p[4].toLongOrNull() ?: return@mapNotNull null,
            status = p[5],
            remainingMinor = p[6].toLongOrNull() ?: return@mapNotNull null,
        )
    }
}

@Composable
private fun SettingsScreen(
    server: String,
    demo: Boolean,
    lockEnabled: Boolean,
    verdict: ConnectionVerdict?,
    connBusy: Boolean,
    onTest: () -> Unit,
    onJars: () -> Unit,
    onBudgets: () -> Unit,
    onCircle: () -> Unit,
    onLockChange: (Boolean) -> Unit,
    onSave: (String, Boolean) -> Unit,
) {
    var s by remember { mutableStateOf(server) }
    var d by remember { mutableStateOf(demo) }
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().padding(20.dp)) {
            Text("Settings", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(8.dp))
            TestChip()
            Spacer(Modifier.height(16.dp))
            OutlinedTextField(
                value = s,
                onValueChange = { s = it },
                label = { Text("Test server", color = Vui.TextSecondary) },
                modifier = Modifier.fillMaxWidth(),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedTextColor = Vui.TextPrimary,
                    unfocusedTextColor = Vui.TextPrimary,
                    focusedBorderColor = Vui.Indigo,
                    unfocusedBorderColor = Vui.Hairline,
                ),
            )
            ConnectionRow(verdict, connBusy, onTest)
            TextButton(onClick = onJars) { Text("Jars", color = Vui.Indigo) }
            TextButton(onClick = onBudgets) { Text("Budgets", color = Vui.Indigo) }
            TextButton(onClick = onCircle) { Text("Circle", color = Vui.Indigo) }
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = lockEnabled, onCheckedChange = onLockChange)
                Text(" Lock with biometrics", color = Vui.TextPrimary)
            }
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = d, onCheckedChange = { d = it })
                Text(" Demo mode (no server)", color = Vui.TextPrimary)
            }
            Spacer(Modifier.weight(1f))
            Button(onClick = { onSave(s, d) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
                Text("Save")
            }
        }
    }
}
