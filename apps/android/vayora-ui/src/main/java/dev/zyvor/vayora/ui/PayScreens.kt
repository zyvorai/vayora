package dev.zyvor.vayora.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.zyvor.vayora.kit.LocalTxn
import dev.zyvor.vayora.kit.Money

@Composable
fun PayHomeScreen(
    balance: Money,
    spentThisMonth: Money,
    recent: List<LocalTxn>,
    hideBalance: Boolean = false,
    onScan: () -> Unit,
    onPaste: () -> Unit,
    onSend: () -> Unit = {},
    onVoice: () -> Unit = {},
    onOpen: (LocalTxn) -> Unit = {},
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 8.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Vayora Pay", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.weight(1f))
                TestChip()
            }
            Spacer(Modifier.height(20.dp))
            GlassCard {
                Text("Test balance", color = Vui.TextSecondary, fontSize = 14.sp, fontWeight = FontWeight.Medium)
                Spacer(Modifier.height(14.dp))
                if (hideBalance) {
                    Text("₹ ••••••", color = Vui.TextPrimary, fontSize = 44.sp, fontWeight = FontWeight.Bold)
                } else {
                    AmountText(balance, 52)
                }
                Spacer(Modifier.height(8.dp))
                Text("Held on this phone, not in a bank", color = Vui.TextTertiary, fontSize = 12.5.sp)
            }
            Spacer(Modifier.height(16.dp))
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(30.dp))
                    .background(Brand.Pay.gradient)
                    .clickable(onClick = onScan)
                    .padding(18.dp)
                    .semantics { contentDescription = "Scan and pay. Opens the camera to scan a payment QR code" },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier
                        .size(62.dp)
                        .clip(CircleShape)
                        .background(Color.White.copy(alpha = 0.18f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Filled.QrCodeScanner, null, tint = Color.White, modifier = Modifier.size(30.dp))
                }
                Column(Modifier.padding(start = 16.dp).weight(1f)) {
                    Text("Scan & pay", color = Color.White, fontSize = 24.sp, fontWeight = FontWeight.Bold)
                    Text("Aim at a Vayora Merchant QR", color = Color.White.copy(alpha = 0.8f), fontSize = 14.sp)
                }
                Icon(Icons.AutoMirrored.Filled.ArrowForward, null, tint = Color.White)
            }
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Quick("Send", Icons.AutoMirrored.Filled.Send, onSend, Modifier.weight(1f))
                Quick("Voice", Icons.Filled.Mic, onVoice, Modifier.weight(1f))
                Quick("Paste link", Icons.Filled.Link, onPaste, Modifier.weight(1f))
            }
            Spacer(Modifier.height(20.dp))
            Text("This month ${spentThisMonth.display}", color = Vui.TextSecondary, fontSize = 14.sp)
            Spacer(Modifier.height(12.dp))
            Text("Recent", color = Vui.TextPrimary, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(8.dp))
            if (recent.isEmpty()) {
                Text("No payments yet on this phone.", color = Vui.TextTertiary, fontSize = 14.sp)
            } else {
                recent.take(8).forEach { t ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(16.dp))
                            .clickable { onOpen(t) }
                            .padding(vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(t.name.ifBlank { t.payee }, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                            Text(t.whenLabel, color = Vui.TextTertiary, fontSize = 12.sp)
                        }
                        Text(
                            (if (t.ok) "−" else "") + t.amount.display,
                            color = if (t.ok) Vui.TextPrimary else Vui.Coral,
                            fontWeight = FontWeight.SemiBold,
                        )
                    }
                }
            }
            Spacer(Modifier.height(32.dp))
        }
    }
}

@Composable
private fun Quick(title: String, icon: ImageVector, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Column(
        modifier
            .clip(RoundedCornerShape(20.dp))
            .background(Color.White.copy(alpha = 0.07f))
            .clickable(onClick = onClick)
            .padding(vertical = 14.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, tint = Vui.Violet)
        Spacer(Modifier.height(6.dp))
        Text(title, color = Vui.TextPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
fun PayReviewScreen(
    name: String,
    payee: String,
    amount: Money?,
    note: String,
    pin: String,
    onPinChange: (String) -> Unit,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
    busy: Boolean = false,
    error: String? = null,
    budgetWarning: String? = null,
    amountText: String = "",
    onAmountTextChange: (String) -> Unit = {},
    circleOptions: List<Pair<String, String>> = emptyList(),
    selectedCircleId: String? = null,
    onSelectCircle: (String?) -> Unit = {},
) {
    val resolved = amount ?: Money.parseDecimal(amountText)
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().padding(20.dp)) {
            Text("Confirm payment", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(8.dp))
            TestChip()
            Spacer(Modifier.height(24.dp))
            GlassCard {
                Text(name.ifBlank { payee }, color = Vui.TextPrimary, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
                Text(payee, color = Vui.TextSecondary, fontSize = 14.sp)
                Spacer(Modifier.height(16.dp))
                if (amount != null) {
                    AmountText(amount, 48)
                } else {
                    OutlinedTextField(
                        value = amountText,
                        onValueChange = onAmountTextChange,
                        label = { Text("Amount ₹", color = Vui.TextSecondary) },
                        modifier = Modifier.fillMaxWidth(),
                        colors = OutlinedTextFieldDefaults.colors(
                            focusedTextColor = Vui.TextPrimary,
                            unfocusedTextColor = Vui.TextPrimary,
                            focusedBorderColor = Vui.Indigo,
                            unfocusedBorderColor = Vui.Hairline,
                        ),
                    )
                }
                if (note.isNotBlank()) {
                    Spacer(Modifier.height(8.dp))
                    Text(note, color = Vui.TextTertiary, fontSize = 13.sp)
                }
            }
            budgetWarning?.let {
                Spacer(Modifier.height(10.dp))
                Text(it, color = Vui.Amber, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
            }
            if (circleOptions.isNotEmpty()) {
                Spacer(Modifier.height(10.dp))
                Text("Pay from", color = Vui.TextSecondary, fontSize = 13.sp)
                TextButton(onClick = { onSelectCircle(null) }) {
                    Text(if (selectedCircleId == null) "● My wallet" else "My wallet", color = Vui.Indigo)
                }
                circleOptions.forEach { (id, label) ->
                    TextButton(onClick = { onSelectCircle(id) }) {
                        Text(if (selectedCircleId == id) "● $label" else label, color = Vui.Indigo)
                    }
                }
            }
            Spacer(Modifier.height(20.dp))
            Text("Test PIN", color = Vui.TextSecondary, fontSize = 14.sp)
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = pin,
                onValueChange = { if (it.length <= 6 && it.all(Char::isDigit)) onPinChange(it) },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                placeholder = { Text("••••", color = Vui.TextTertiary) },
                colors = OutlinedTextFieldDefaults.colors(
                    focusedTextColor = Vui.TextPrimary,
                    unfocusedTextColor = Vui.TextPrimary,
                    focusedBorderColor = Vui.Indigo,
                    unfocusedBorderColor = Vui.Hairline,
                    cursorColor = Vui.Indigo,
                ),
            )
            if (error != null) {
                Spacer(Modifier.height(8.dp))
                Text(error, color = Vui.Coral, fontSize = 14.sp)
            }
            Spacer(Modifier.weight(1f))
            Button(
                onClick = onConfirm,
                enabled = !busy && pin.length >= 4 && resolved != null,
                modifier = Modifier.fillMaxWidth().height(56.dp),
                shape = RoundedCornerShape(26.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Vui.Indigo, contentColor = Color.White),
            ) {
                Text(if (busy) "Paying…" else "Slide to pay · confirm", fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
            }
            TextButton(onClick = onCancel, modifier = Modifier.align(Alignment.CenterHorizontally)) {
                Text("Cancel", color = Vui.TextSecondary)
            }
        }
    }
}

@Composable
fun PasteLinkScreen(
    value: String,
    onValueChange: (String) -> Unit,
    onContinue: () -> Unit,
    onCancel: () -> Unit,
    error: String? = null,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().padding(20.dp)) {
            Text("Paste a UPI link", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(16.dp))
            OutlinedTextField(
                value = value,
                onValueChange = onValueChange,
                modifier = Modifier.fillMaxWidth().height(140.dp),
                placeholder = { Text("upi://pay?pa=…", color = Vui.TextTertiary) },
                colors = OutlinedTextFieldDefaults.colors(
                    focusedTextColor = Vui.TextPrimary,
                    unfocusedTextColor = Vui.TextPrimary,
                    focusedBorderColor = Vui.Indigo,
                    unfocusedBorderColor = Vui.Hairline,
                ),
            )
            if (error != null) {
                Spacer(Modifier.height(8.dp))
                Text(error, color = Vui.Coral)
            }
            Spacer(Modifier.weight(1f))
            Button(
                onClick = onContinue,
                modifier = Modifier.fillMaxWidth().height(56.dp),
                shape = RoundedCornerShape(26.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Vui.Indigo),
            ) { Text("Continue", fontWeight = FontWeight.SemiBold) }
            TextButton(onClick = onCancel) { Text("Cancel", color = Vui.TextSecondary) }
        }
    }
}
