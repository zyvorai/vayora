package dev.zyvor.vayora.ui

import android.graphics.Bitmap
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import dev.zyvor.vayora.kit.BudgetStatus
import dev.zyvor.vayora.kit.Budgets
import dev.zyvor.vayora.kit.CircleRecord
import dev.zyvor.vayora.kit.CircleRole
import dev.zyvor.vayora.kit.ClosingInfo
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.SpendCategory

@Composable
fun ReceiveScreen(
    name: String,
    vpa: String,
    link: String,
    onBack: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Indigo) }
                Text("Receive", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            }
            Spacer(Modifier.height(16.dp))
            val bmp = remember(link) {
                runCatching {
                    val hints = mapOf(EncodeHintType.MARGIN to 1)
                    val matrix = QRCodeWriter().encode(link, BarcodeFormat.QR_CODE, 512, 512, hints)
                    Bitmap.createBitmap(512, 512, Bitmap.Config.ARGB_8888).also { b ->
                        for (x in 0 until 512) for (y in 0 until 512) {
                            b.setPixel(x, y, if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
                        }
                    }
                }.getOrNull()
            }
            Box(
                Modifier
                    .size(240.dp)
                    .clip(RoundedCornerShape(24.dp))
                    .background(Color.White)
                    .padding(14.dp),
                contentAlignment = Alignment.Center,
            ) {
                bmp?.let { Image(it.asImageBitmap(), contentDescription = "Receive QR", modifier = Modifier.fillMaxSize()) }
            }
            Spacer(Modifier.height(12.dp))
            Text(name.ifBlank { "Your test UPI ID" }, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold, fontSize = 20.sp)
            Text(vpa, color = Vui.TextSecondary, fontSize = 14.sp)
            Text(
                "Another Vayora Pay scans this and types the amount. Test wallets only.",
                color = Vui.TextTertiary,
                fontSize = 12.sp,
                modifier = Modifier.padding(top = 12.dp),
            )
        }
    }
}

@Composable
fun BudgetsScreen(
    budgets: Budgets,
    statuses: List<BudgetStatus>,
    onSet: (SpendCategory, Long?) -> Unit,
    onBack: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Indigo) }
                Text("Budgets", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            }
            Text("Monthly limits. A warning on review never blocks a payment.", color = Vui.TextSecondary, fontSize = 13.sp)
            SpendCategory.editable.forEach { c ->
                var text by remember(budgets.limit(c)) {
                    mutableStateOf(budgets.limit(c)?.let { Money(it).decimalString } ?: "")
                }
                GlassCard(Modifier.padding(top = 10.dp).fillMaxWidth()) {
                    Text(c.title, color = Vui.TextPrimary, fontWeight = FontWeight.Bold)
                    statuses.firstOrNull { it.category == c }?.let {
                        Text("${Money(it.spentMinor).display} of ${Money(it.limitMinor).display}", color = Vui.TextSecondary, fontSize = 13.sp)
                    }
                    OutlinedTextField(
                        value = text,
                        onValueChange = { text = it },
                        label = { Text("Limit ₹", color = Vui.TextSecondary) },
                        modifier = Modifier.fillMaxWidth(),
                        colors = fieldColors(),
                    )
                    TextButton(onClick = {
                        onSet(c, Money.parseDecimal(text)?.minor)
                    }) { Text("Save", color = Vui.Indigo) }
                }
            }
        }
    }
}

@Composable
fun CircleScreen(
    circles: List<CircleRecord>,
    message: String?,
    showQr: String?,
    onCreate: (delegate: String, per: Money, total: Money) -> Unit,
    onRevoke: (CircleRecord) -> Unit,
    onScan: () -> Unit,
    onBack: () -> Unit,
) {
    var delegate by remember { mutableStateOf("Friend") }
    var per by remember { mutableStateOf("500.00") }
    var total by remember { mutableStateOf("2000.00") }
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Indigo) }
                Text("Circle", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.weight(1f))
                TestChip()
            }
            message?.let { Text(it, color = Vui.Amber, fontSize = 13.sp) }
            Text("Lend your test wallet up to a per-payment and total limit (simulator).", color = Vui.TextSecondary, fontSize = 13.sp)
            GlassCard(Modifier.padding(vertical = 10.dp).fillMaxWidth()) {
                OutlinedTextField(delegate, { delegate = it }, label = { Text("Delegate name") }, modifier = Modifier.fillMaxWidth(), colors = fieldColors())
                OutlinedTextField(per, { per = it }, label = { Text("Per payment ₹") }, modifier = Modifier.fillMaxWidth(), colors = fieldColors())
                OutlinedTextField(total, { total = it }, label = { Text("Total ₹") }, modifier = Modifier.fillMaxWidth(), colors = fieldColors())
                Button(
                    onClick = {
                        val p = Money.parseDecimal(per) ?: return@Button
                        val t = Money.parseDecimal(total) ?: return@Button
                        onCreate(delegate, p, t)
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = Vui.Indigo),
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                ) { Text("New circle") }
            }
            TextButton(onClick = onScan) { Text("Scan to join", color = Vui.Indigo) }
            showQr?.let { uri ->
                Text("Show this QR to the delegate", color = Vui.TextSecondary, fontSize = 13.sp)
                val bmp = remember(uri) {
                    runCatching {
                        val hints = mapOf(EncodeHintType.MARGIN to 1)
                        val matrix = QRCodeWriter().encode(uri, BarcodeFormat.QR_CODE, 512, 512, hints)
                        Bitmap.createBitmap(512, 512, Bitmap.Config.ARGB_8888).also { b ->
                            for (x in 0 until 512) for (y in 0 until 512) {
                                b.setPixel(x, y, if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
                            }
                        }
                    }.getOrNull()
                }
                Box(
                    Modifier
                        .padding(vertical = 8.dp)
                        .size(220.dp)
                        .clip(RoundedCornerShape(20.dp))
                        .background(Color.White)
                        .padding(12.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    bmp?.let { Image(it.asImageBitmap(), contentDescription = "Circle QR", modifier = Modifier.fillMaxSize()) }
                }
                Text(uri, color = Vui.TextTertiary, fontSize = 11.sp)
            }
            circles.forEach { c ->
                GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                    Text(if (c.role == CircleRole.Primary) "Lending to ${c.other}" else "Paying from ${c.other}", color = Vui.TextPrimary, fontWeight = FontWeight.Bold)
                    Text(c.status, color = Vui.TextSecondary)
                    Text("Up to ${Money(c.perPaymentMinor).display} · ${Money(c.remainingMinor).display} left", color = Vui.TextTertiary, fontSize = 12.sp)
                    if (c.role == CircleRole.Primary && c.isLive) {
                        TextButton(onClick = { onRevoke(c) }) { Text("Take back", color = Vui.Coral) }
                    }
                }
            }
        }
    }
}

@Composable
fun VoicePayScreen(
    hearing: Boolean,
    transcript: String,
    message: String?,
    onListen: () -> Unit,
    onBack: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Indigo) }
                Text("Voice pay", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            }
            Spacer(Modifier.height(24.dp))
            Text("Say “Pay Asha 200 rupees” or “Asha ko 300 rupaye bhejo”.", color = Vui.TextSecondary)
            Spacer(Modifier.height(16.dp))
            if (transcript.isNotBlank()) Text(transcript, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
            message?.let { Text(it, color = Vui.Coral, modifier = Modifier.padding(top = 8.dp)) }
            Spacer(Modifier.weight(1f))
            Button(
                onClick = onListen,
                enabled = !hearing,
                modifier = Modifier.fillMaxWidth().height(56.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Vui.Indigo),
            ) { Text(if (hearing) "Listening…" else "Listen") }
        }
    }
}

@Composable
fun EndOfDayScreen(
    branches: List<Pair<String, String>>,
    selectedBranchId: String?,
    closing: ClosingInfo?,
    message: String?,
    busy: Boolean,
    onSelectBranch: (String) -> Unit,
    onCreateBranch: (String) -> Unit,
    onAddCash: (kind: String, amount: Money) -> Unit,
    onClose: (counted: Money) -> Unit,
    onBack: () -> Unit,
) {
    var branchName by remember { mutableStateOf("Main") }
    var cashText by remember { mutableStateOf("100.00") }
    var countedText by remember { mutableStateOf("") }
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Emerald) }
                Text("End of day", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            }
            message?.let { Text(it, color = Vui.Amber, fontSize = 13.sp) }
            Text("Record cash and close the till. A second person signs off on the console.", color = Vui.TextSecondary, fontSize = 13.sp)
            Spacer(Modifier.height(8.dp))
            branches.forEach { (id, name) ->
                TextButton(onClick = { onSelectBranch(id) }) {
                    Text(if (id == selectedBranchId) "● $name" else name, color = Vui.Emerald)
                }
            }
            Row {
                OutlinedTextField(branchName, { branchName = it }, label = { Text("New branch") }, modifier = Modifier.weight(1f), colors = fieldColors(Vui.Emerald))
                TextButton(onClick = { onCreateBranch(branchName) }) { Text("Add", color = Vui.Emerald) }
            }
            OutlinedTextField(cashText, { cashText = it }, label = { Text("Cash ₹") }, modifier = Modifier.fillMaxWidth(), colors = fieldColors(Vui.Emerald))
            Row {
                listOf("sale", "refund", "float").forEach { kind ->
                    TextButton(onClick = {
                        Money.parseDecimal(cashText)?.let { onAddCash(kind, it) }
                    }) { Text(kind.replaceFirstChar { it.uppercase() }, color = Vui.Emerald) }
                }
            }
            OutlinedTextField(countedText, { countedText = it }, label = { Text("Counted till ₹") }, modifier = Modifier.fillMaxWidth(), colors = fieldColors(Vui.Emerald))
            Button(
                onClick = { Money.parseDecimal(countedText)?.let(onClose) },
                enabled = !busy && selectedBranchId != null,
                modifier = Modifier.fillMaxWidth().height(52.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Vui.Emerald),
            ) { Text(if (busy) "…" else "Close the day") }
            closing?.let { c ->
                GlassCard(Modifier.padding(top = 12.dp).fillMaxWidth()) {
                    Text(c.verdict, color = Vui.TextPrimary, fontWeight = FontWeight.Bold, fontSize = 20.sp)
                    Text("Expected ${Money(c.expectedCashMinor).display}", color = Vui.TextSecondary)
                    Text("Counted ${Money(c.countedCashMinor).display}", color = Vui.TextSecondary)
                    Text("Difference ${Money(c.differenceMinor).display}", color = Vui.TextSecondary)
                    Text(c.status, color = Vui.TextTertiary, fontSize = 12.sp)
                    c.digital.forEach { d ->
                        Text("${d.method}: ${Money(d.netMinor).display} (${d.payments})", color = Vui.TextTertiary, fontSize = 12.sp)
                    }
                }
            }
        }
    }
}

@Composable
private fun fieldColors(accent: androidx.compose.ui.graphics.Color = Vui.Indigo) =
    OutlinedTextFieldDefaults.colors(
        focusedTextColor = Vui.TextPrimary,
        unfocusedTextColor = Vui.TextPrimary,
        focusedBorderColor = accent,
        unfocusedBorderColor = Vui.Hairline,
    )
