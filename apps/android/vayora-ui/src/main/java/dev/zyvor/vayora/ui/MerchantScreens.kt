package dev.zyvor.vayora.ui

import android.graphics.Bitmap
import androidx.compose.foundation.Image
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Backspace
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.PaymentIntent

enum class PaymentRowState { Succeeded, Processing, Refunded, PartRefunded, Failed }

data class PaymentRow(
    val id: String,
    val amount: Money,
    val whenLabel: String,
    val note: String = "",
    val state: PaymentRowState,
)

@Composable
fun CollectKeypadScreen(
    text: String,
    note: String,
    chips: List<String> = listOf("Tea", "Lunch", "Deposit", "Other"),
    canContinue: Boolean,
    onDigit: (Int) -> Unit,
    onPoint: () -> Unit,
    onDelete: () -> Unit,
    onChip: (String) -> Unit,
    onShowQr: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(Modifier.fillMaxSize().padding(horizontal = 20.dp, vertical = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Collect", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.weight(1f))
                TestChip()
            }
            Spacer(Modifier.weight(1f))
            Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.Center, modifier = Modifier.fillMaxWidth()) {
                Text("₹", color = Vui.TextSecondary, fontSize = 40.sp, fontWeight = FontWeight.SemiBold)
                Text(
                    if (text.isEmpty()) "0" else text,
                    color = if (text.isEmpty()) Vui.TextTertiary else Vui.TextPrimary,
                    fontSize = 72.sp,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                )
            }
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
                chips.forEach { c ->
                    val on = note == c
                    Text(
                        c,
                        modifier = Modifier
                            .clip(RoundedCornerShape(999.dp))
                            .background(if (on) Brand.Merchant.glow else Color.White.copy(alpha = 0.10f))
                            .clickable { onChip(c) }
                            .padding(horizontal = 14.dp, vertical = 8.dp),
                        color = if (on) Vui.Ink else Vui.TextPrimary,
                        fontWeight = FontWeight.SemiBold,
                        fontSize = 14.sp,
                    )
                }
            }
            Spacer(Modifier.weight(1f))
            KeyPad(onDigit, onPoint, onDelete)
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = onShowQr,
                enabled = canContinue,
                modifier = Modifier.fillMaxWidth().height(56.dp),
                shape = RoundedCornerShape(26.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = Brand.Merchant.glow,
                    contentColor = Vui.Ink,
                    disabledContainerColor = Color.White.copy(alpha = 0.08f),
                    disabledContentColor = Vui.TextTertiary,
                ),
            ) {
                Text("Show QR", fontWeight = FontWeight.SemiBold, fontSize = 19.sp)
            }
        }
    }
}

@Composable
private fun KeyPad(onDigit: (Int) -> Unit, onPoint: () -> Unit, onDelete: () -> Unit) {
    val rows = listOf(
        listOf("1", "2", "3"),
        listOf("4", "5", "6"),
        listOf("7", "8", "9"),
        listOf(".", "0", "⌫"),
    )
    rows.forEach { row ->
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            row.forEach { key ->
                Box(
                    Modifier
                        .weight(1f)
                        .height(58.dp)
                        .clip(RoundedCornerShape(18.dp))
                        .background(Color.White.copy(alpha = 0.07f))
                        .clickable {
                            when (key) {
                                "." -> onPoint()
                                "⌫" -> onDelete()
                                else -> onDigit(key.toInt())
                            }
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    if (key == "⌫") Icon(Icons.Filled.Backspace, null, tint = Vui.TextPrimary)
                    else Text(key, color = Vui.TextPrimary, fontSize = 24.sp, fontWeight = FontWeight.SemiBold)
                }
            }
        }
        Spacer(Modifier.height(8.dp))
    }
}

@Composable
fun QrHeroScreen(
    amount: Money,
    upiUri: String?,
    note: String,
    paid: Boolean,
    onCancel: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(
            Modifier.fillMaxSize().padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(if (paid) "Received" else "Waiting for payment", color = Vui.TextPrimary, fontSize = 24.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(8.dp))
            AmountText(amount, 40)
            if (note.isNotBlank()) Text(note, color = Vui.TextSecondary, fontSize = 14.sp)
            Spacer(Modifier.height(24.dp))
            Box(
                Modifier
                    .size(260.dp)
                    .clip(RoundedCornerShape(28.dp))
                    .background(if (paid) Vui.Emerald.copy(alpha = 0.2f) else Color.White)
                    .padding(16.dp),
                contentAlignment = Alignment.Center,
            ) {
                when {
                    paid -> Text("✓", color = Vui.Emerald, fontSize = 72.sp, fontWeight = FontWeight.Bold)
                    upiUri != null -> {
                        val bmp = rememberQrBitmap(upiUri, 512)
                        if (bmp != null) Image(bmp.asImageBitmap(), contentDescription = "Payment QR", modifier = Modifier.fillMaxSize())
                    }
                    else -> Text("Creating…", color = Vui.Ink, fontWeight = FontWeight.SemiBold)
                }
            }
            Spacer(Modifier.height(16.dp))
            Text(
                if (paid) "Test money received on the simulator." else "Customer scans with Vayora Pay (test mode).",
                color = Vui.TextSecondary,
                textAlign = TextAlign.Center,
                fontSize = 14.sp,
            )
            Spacer(Modifier.weight(1f))
            TextButton(onClick = onCancel) {
                Text(if (paid) "Done" else "Cancel", color = Vui.TextSecondary)
            }
        }
    }
}

@Composable
private fun rememberQrBitmap(content: String, size: Int): Bitmap? {
    return androidx.compose.runtime.remember(content, size) {
        runCatching {
            val hints = mapOf(EncodeHintType.MARGIN to 1)
            val matrix = QRCodeWriter().encode(content, BarcodeFormat.QR_CODE, size, size, hints)
            val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
            for (x in 0 until size) for (y in 0 until size) {
                bmp.setPixel(x, y, if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
            }
            bmp
        }.getOrNull()
    }
}

@Composable
fun PaymentsListScreen(rows: List<PaymentRow>, onOpen: (PaymentRow) -> Unit = {}) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(Modifier.fillMaxSize().padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Payments", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.weight(1f))
                TestChip()
            }
            Spacer(Modifier.height(16.dp))
            if (rows.isEmpty()) {
                Text("No payments yet. Collect on the first tab.", color = Vui.TextTertiary)
            } else {
                LazyColumn {
                    items(rows, key = { it.id }) { r ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(16.dp))
                                .clickable { onOpen(r) }
                                .padding(vertical = 14.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(r.amount.display, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                                Text(r.whenLabel + if (r.note.isNotBlank()) " · ${r.note}" else "", color = Vui.TextTertiary, fontSize = 12.sp)
                            }
                            Text(
                                when (r.state) {
                                    PaymentRowState.Succeeded -> "Paid"
                                    PaymentRowState.Processing -> "Waiting"
                                    PaymentRowState.Refunded -> "Refunded"
                                    PaymentRowState.PartRefunded -> "Part refunded"
                                    PaymentRowState.Failed -> "Failed"
                                },
                                color = when (r.state) {
                                    PaymentRowState.Succeeded -> Vui.Emerald
                                    PaymentRowState.Failed -> Vui.Coral
                                    else -> Vui.Amber
                                },
                                fontSize = 13.sp,
                                fontWeight = FontWeight.SemiBold,
                            )
                        }
                    }
                }
            }
        }
    }
}

fun PaymentIntent.toRow(): PaymentRow = PaymentRow(
    id = id,
    amount = money,
    whenLabel = createdAt?.take(16)?.replace('T', ' ') ?: id.take(12),
    state = when (status) {
        "succeeded", "captured" -> PaymentRowState.Succeeded
        "requires_payment_method", "processing" -> PaymentRowState.Processing
        else -> PaymentRowState.Processing
    },
)
