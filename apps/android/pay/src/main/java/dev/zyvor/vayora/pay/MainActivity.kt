package dev.zyvor.vayora.pay

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
import androidx.compose.runtime.Composable
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
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.TestWallet
import dev.zyvor.vayora.kit.UPIPayload
import dev.zyvor.vayora.kit.VayoraClient
import dev.zyvor.vayora.ui.AmbientBackground
import dev.zyvor.vayora.ui.Brand
import dev.zyvor.vayora.ui.PasteLinkScreen
import dev.zyvor.vayora.ui.PayHomeScreen
import dev.zyvor.vayora.ui.PayReviewScreen
import dev.zyvor.vayora.ui.ScannerScreen
import dev.zyvor.vayora.ui.TestChip
import dev.zyvor.vayora.ui.Vui
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
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
    data class Review(val payload: UPIPayload, val raw: String) : PayRoute
}

@Composable
private fun PayApp() {
    val ctx = LocalContext.current
    val prefs = remember { ctx.getSharedPreferences("vayora_pay", Context.MODE_PRIVATE) }
    val wallet = remember { TestWallet(prefs.getLong("balance", 50_000_00L)) }
    var tab by remember { mutableIntStateOf(0) }
    var route by remember { mutableStateOf<PayRoute>(PayRoute.Home) }
    var paste by remember { mutableStateOf("") }
    var pasteError by remember { mutableStateOf<String?>(null) }
    var pin by remember { mutableStateOf("") }
    var payError by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var tick by remember { mutableIntStateOf(0) }
    val scope = rememberCoroutineScope()

    fun server() = prefs.getString("server", "http://10.0.2.2:8080") ?: "http://10.0.2.2:8080"
    fun demo() = prefs.getBoolean("demo", true)
    fun refresh() {
        prefs.edit().putLong("balance", wallet.balanceMinor).apply()
        tick++
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
                    onSave = { s, d ->
                        prefs.edit().putString("server", s).putBoolean("demo", d).apply()
                    },
                )
                PayRoute.Scan -> ScannerScreen(
                    brand = Brand.Pay,
                    hint = "Aim at a Vayora Merchant QR",
                    onCode = { raw ->
                        try {
                            route = PayRoute.Review(UPIPayload.parse(raw), raw)
                            paste = raw
                            pin = ""
                            payError = null
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
                    },
                    onCancel = { route = PayRoute.Home; tab = 0 },
                )
                PayRoute.Paste -> PasteLinkScreen(
                    value = paste,
                    onValueChange = { paste = it; pasteError = null },
                    onContinue = {
                        try {
                            route = PayRoute.Review(UPIPayload.parse(paste), paste)
                            pin = ""
                            payError = null
                        } catch (e: UPIPayload.Failure) {
                            pasteError = when (e) {
                                UPIPayload.Failure.NotUPI -> "Not a upi://pay link."
                                UPIPayload.Failure.MissingPayee, UPIPayload.Failure.BadPayee -> "That UPI ID looks wrong."
                                UPIPayload.Failure.BadAmount -> "The amount is not valid."
                                UPIPayload.Failure.UnsupportedCurrency -> "Only INR is supported."
                            }
                        }
                    },
                    onCancel = { route = PayRoute.Home; tab = 0 },
                    error = pasteError,
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
private fun SettingsScreen(server: String, demo: Boolean, onSave: (String, Boolean) -> Unit) {
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
            Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = d, onCheckedChange = { d = it })
                Text(" Demo mode (no server)", color = Vui.TextPrimary)
            }
            Text(
                "Demo keeps a test wallet on this phone. Turn it off and point at a sandbox with ZYVOR_SIM_PAYER=1 to approve simulator QRs.",
                color = Vui.TextTertiary,
                fontSize = 13.sp,
                modifier = Modifier.padding(top = 8.dp),
            )
            Spacer(Modifier.weight(1f))
            Button(onClick = { onSave(s, d) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
                Text("Save")
            }
        }
    }
}
