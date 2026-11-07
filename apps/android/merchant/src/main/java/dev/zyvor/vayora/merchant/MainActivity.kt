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
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.VayoraClient
import dev.zyvor.vayora.ui.AmbientBackground
import dev.zyvor.vayora.ui.Brand
import dev.zyvor.vayora.ui.CollectKeypadScreen
import dev.zyvor.vayora.ui.PaymentRow
import dev.zyvor.vayora.ui.PaymentRowState
import dev.zyvor.vayora.ui.PaymentsListScreen
import dev.zyvor.vayora.ui.QrHeroScreen
import dev.zyvor.vayora.ui.TestChip
import dev.zyvor.vayora.ui.Vui
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
    data object Settings : Route
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
            runCatching {
                withContext(Dispatchers.IO) { client().listPayments() }
            }.onSuccess { page ->
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

    Scaffold(
        containerColor = Vui.Ink,
        bottomBar = {
            if (route !is Route.Qr) {
                NavigationBar(containerColor = Vui.InkRaised) {
                    listOf(
                        Triple(0, "Collect", Icons.Filled.QrCode),
                        Triple(1, "Payments", Icons.Filled.Payments),
                        Triple(2, "Settings", Icons.Filled.Settings),
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
                                    2 -> Route.Settings
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
                                        val a = client().createAttempt(i.id)
                                        i to a
                                    }
                                    route = Route.Qr(money, note, intent.id, attempt.upiURI, false)
                                } catch (e: Exception) {
                                    error = e.message
                                }
                            }
                        }
                    },
                )
                Route.Payments -> PaymentsListScreen(rows)
                Route.Settings -> SettingsScreen(
                    server = server(),
                    key = key(),
                    demo = demo(),
                    onSave = { s, k, d ->
                        prefs.edit().putString("server", s).putString("key", k).putBoolean("demo", d).apply()
                    },
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
            Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = d, onCheckedChange = { d = it })
                Text(" Demo mode (no server)", color = Vui.TextPrimary)
            }
            Text(
                "Demo invents a QR and marks it paid after a few seconds. Against a real sandbox, set the server and a test key; the QR is the attempt’s upi:// payload.",
                color = Vui.TextTertiary,
                fontSize = 13.sp,
                modifier = Modifier.padding(top = 8.dp),
            )
            Spacer(Modifier.weight(1f))
            Button(onClick = { onSave(s, k, d) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
                Text("Save")
            }
        }
    }
}
