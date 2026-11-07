package dev.zyvor.vayora.pay

import android.content.Context
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
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
import dev.zyvor.vayora.kit.ConnectionVerdict
import dev.zyvor.vayora.kit.JarBook
import dev.zyvor.vayora.kit.LockPolicy
import dev.zyvor.vayora.kit.MandatePayload
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.ScamHints
import dev.zyvor.vayora.kit.TestWallet
import dev.zyvor.vayora.kit.UPIPayload
import dev.zyvor.vayora.kit.VayoraClient
import dev.zyvor.vayora.ui.AmbientBackground
import dev.zyvor.vayora.ui.Brand
import dev.zyvor.vayora.ui.ConnectionRow
import dev.zyvor.vayora.ui.JarsScreen
import dev.zyvor.vayora.ui.MandateApproveScreen
import dev.zyvor.vayora.ui.PasteLinkScreen
import dev.zyvor.vayora.ui.PayHomeScreen
import dev.zyvor.vayora.ui.PayReviewScreen
import dev.zyvor.vayora.ui.ScannerScreen
import dev.zyvor.vayora.ui.TestChip
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
    data object Jars : PayRoute
    data class Review(val payload: UPIPayload, val raw: String) : PayRoute
    data class Mandate(val payload: MandatePayload, val raw: String) : PayRoute
}

@Composable
private fun PayApp() {
    val ctx = LocalContext.current
    val activity = ctx as FragmentActivity
    val prefs = remember { ctx.getSharedPreferences("vayora_pay", Context.MODE_PRIVATE) }
    val wallet = remember { TestWallet(prefs.getLong("balance", 50_000_00L)) }
    val jars = remember { JarBook() }
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
                    onLockChange = {
                        lockEnabled = it
                        prefs.edit().putBoolean("lock", it).apply()
                        if (it) locked = true
                    },
                    onSave = { s, d ->
                        prefs.edit().putString("server", s).putBoolean("demo", d).apply()
                    },
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
                PayRoute.Scan -> ScannerScreen(
                    brand = Brand.Pay,
                    hint = "Aim at a pay or AutoPay QR",
                    onCode = { raw ->
                        when {
                            raw.lowercase().startsWith("upi://mandate?") -> {
                                try {
                                    route = PayRoute.Mandate(MandatePayload.parse(raw), raw)
                                    payError = null
                                } catch (e: MandatePayload.Failure) {
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
                    onCancel = { route = PayRoute.Home; tab = 0 },
                )
                PayRoute.Paste -> PasteLinkScreen(
                    value = paste,
                    onValueChange = { paste = it; pasteError = null },
                    onContinue = {
                        try {
                            if (paste.lowercase().startsWith("upi://mandate?")) {
                                route = PayRoute.Mandate(MandatePayload.parse(paste), paste)
                            } else {
                                val p = UPIPayload.parse(paste)
                                route = PayRoute.Review(p, paste)
                                pin = ""
                                payError = ScamHints.check(p.name, p.note)
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
                    PayReviewScreen(
                        name = r.payload.name,
                        payee = r.payload.payee,
                        amount = amount,
                        note = r.payload.note,
                        pin = pin,
                        onPinChange = { pin = it },
                        busy = busy,
                        error = payError,
                        onCancel = { route = PayRoute.Home; tab = 0 },
                        onConfirm = {
                            busy = true
                            payError = null
                            scope.launch {
                                try {
                                    if (!demo() && r.payload.payee.contains("zyvor-sim")) {
                                        withContext(Dispatchers.IO) {
                                            VayoraClient(server()).simPay(r.raw)
                                        }
                                    }
                                    wallet.debit(r.payload.name.ifBlank { r.payload.payee }, r.payload.payee, amount)
                                    refresh()
                                    route = PayRoute.Home
                                    tab = 0
                                    paste = ""
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

@Composable
private fun SettingsScreen(
    server: String,
    demo: Boolean,
    lockEnabled: Boolean,
    verdict: ConnectionVerdict?,
    connBusy: Boolean,
    onTest: () -> Unit,
    onJars: () -> Unit,
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
