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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.zyvor.vayora.kit.Ageing
import dev.zyvor.vayora.kit.Cashflow
import dev.zyvor.vayora.kit.ConnectionVerdict
import dev.zyvor.vayora.kit.DisputeRow
import dev.zyvor.vayora.kit.GstSummary
import dev.zyvor.vayora.kit.MandateInfo
import dev.zyvor.vayora.kit.Money
import dev.zyvor.vayora.kit.PendingApproval
import dev.zyvor.vayora.kit.ReconCase
import dev.zyvor.vayora.kit.ReminderDraft
import dev.zyvor.vayora.kit.SettlementInfo

@Composable
fun ConnectionRow(
    verdict: ConnectionVerdict?,
    busy: Boolean,
    onTest: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Test connection", color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            Button(
                onClick = onTest,
                enabled = !busy,
                colors = ButtonDefaults.buttonColors(containerColor = Vui.Indigo),
            ) { Text(if (busy) "…" else "Test") }
        }
        verdict?.let {
            Spacer(Modifier.height(8.dp))
            Text(it.title, color = if (it.isOk) Vui.Emerald else Vui.Coral, fontWeight = FontWeight.Bold)
            Text(it.advice(), color = Vui.TextSecondary, fontSize = 13.sp)
        }
    }
}

@Composable
fun OwedScreen(
    ageing: Ageing?,
    drafts: List<ReminderDraft>,
    approvals: List<PendingApproval>,
    disputes: List<DisputeRow>,
    cases: List<ReconCase>,
    busy: Boolean,
    message: String?,
    onRefresh: () -> Unit,
    onApproveDraft: (ReminderDraft) -> Unit,
    onRejectDraft: (ReminderDraft) -> Unit,
    onEmailInvoice: (String) -> Unit,
    onDecide: (PendingApproval, Boolean) -> Unit,
    onResolveCase: (ReconCase) -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(20.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Owed", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.weight(1f))
                TestChip()
            }
            TextButton(onClick = onRefresh, enabled = !busy) { Text("Refresh", color = Vui.Emerald) }
            message?.let { Text(it, color = Vui.Amber, fontSize = 13.sp, modifier = Modifier.padding(bottom = 8.dp)) }

            if (approvals.isNotEmpty()) {
                SectionTitle("Waiting for your approval")
                approvals.forEach { a ->
                    GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                        Text(a.title, color = Vui.TextPrimary, fontWeight = FontWeight.Bold)
                        AmountText(Money(a.amountMinor, a.currency), 22)
                        if (a.detail.isNotBlank()) Text(a.detail, color = Vui.TextSecondary, fontSize = 13.sp)
                        Text("Requested by ${a.requestedBy.ifBlank { "someone" }}", color = Vui.TextTertiary, fontSize = 12.sp)
                        Row {
                            TextButton(onClick = { onDecide(a, true) }) { Text("Approve", color = Vui.Emerald) }
                            TextButton(onClick = { onDecide(a, false) }) { Text("Reject", color = Vui.Coral) }
                        }
                    }
                }
            }

            if (drafts.isNotEmpty()) {
                SectionTitle("Reminders to approve")
                drafts.forEach { d ->
                    GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                        Text(d.invoiceNumber ?: d.id, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                        Text(d.message, color = Vui.TextSecondary, fontSize = 13.sp)
                        Row {
                            TextButton(onClick = { onApproveDraft(d) }) { Text("Approve", color = Vui.Emerald) }
                            TextButton(onClick = { onRejectDraft(d) }) { Text("Reject", color = Vui.Coral) }
                        }
                    }
                }
            }

            val total = ageing?.inrTotalMinor ?: 0
            SectionTitle("Unpaid · ${Money(total).display}")
            ageing?.inrBuckets.orEmpty().forEach { bucket ->
                GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                    Text(bucket.title, color = Vui.TextPrimary, fontWeight = FontWeight.Bold)
                    AmountText(Money(bucket.outstandingMinor), 20)
                    bucket.invoices.take(5).forEach { inv ->
                        Row(
                            Modifier.fillMaxWidth().padding(top = 6.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(inv.number, color = Vui.TextPrimary, fontSize = 14.sp)
                                Text(Money(inv.outstandingMinor).display, color = Vui.TextSecondary, fontSize = 12.sp)
                            }
                            TextButton(onClick = { onEmailInvoice(inv.id) }) { Text("Email", color = Vui.Emerald) }
                        }
                    }
                }
            }

            if (disputes.isNotEmpty()) {
                SectionTitle("Open disputes")
                disputes.forEach { d ->
                    GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                        Text(d.reason ?: d.id, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                        AmountText(Money(d.amountMinor, d.currency), 18)
                        d.respondBy?.let { Text("Respond by $it", color = Vui.Amber, fontSize = 12.sp) }
                    }
                }
            }

            if (cases.isNotEmpty()) {
                SectionTitle("Money to check")
                cases.forEach { c ->
                    GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                        Text(c.summary ?: c.kind ?: c.id, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                        if (c.gapMinor != 0L) AmountText(Money(kotlin.math.abs(c.gapMinor)), 18)
                        TextButton(onClick = { onResolveCase(c) }) { Text("Mark checked", color = Vui.Emerald) }
                    }
                }
            }

            if (ageing == null && approvals.isEmpty() && drafts.isEmpty() && !busy) {
                Text("Pull Refresh after you set a test server and key.", color = Vui.TextTertiary)
            }
        }
    }
}

@Composable
fun CashflowScreen(
    cashflow: Cashflow?,
    settlements: List<SettlementInfo>,
    gst: GstSummary?,
    gstPeriod: String,
    onBack: () -> Unit,
    onPrevMonth: () -> Unit,
    onNextMonth: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(20.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Emerald) }
                Text("Cashflow", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold, modifier = Modifier.padding(start = 8.dp))
            }
            Spacer(Modifier.height(12.dp))
            cashflow?.let { cf ->
                GlassCard(Modifier.fillMaxWidth()) {
                    Text("Still to come in", color = Vui.TextSecondary, fontSize = 13.sp)
                    AmountText(Money(cf.totalOwedMinor), 36)
                    Spacer(Modifier.height(8.dp))
                    Stat("Overdue", cf.overdueMinor)
                    Stat("Promised", cf.promisedMinor)
                    Stat("Payouts waiting", cf.payoutsWaitingMinor)
                }
                Spacer(Modifier.height(12.dp))
                SectionTitle("Due by week")
                cf.weeks.forEach { w ->
                    Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text(w.starts, color = Vui.TextSecondary)
                        Text(Money(w.dueMinor).display, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
            SectionTitle("GST · $gstPeriod")
            Row {
                TextButton(onClick = onPrevMonth) { Text("←", color = Vui.Emerald) }
                TextButton(onClick = onNextMonth) { Text("→", color = Vui.Emerald) }
            }
            gst?.let { g ->
                GlassCard(Modifier.fillMaxWidth()) {
                    Stat("Taxable", g.taxableMinor)
                    Stat("CGST", g.cgstMinor)
                    Stat("SGST", g.sgstMinor)
                    Stat("IGST", g.igstMinor)
                    Stat("Tax total", g.taxTotalMinor)
                    Text("A summary to prepare your return from. Zyvor does not file it.", color = Vui.TextTertiary, fontSize = 11.sp)
                }
            } ?: Text("No GST figures for this month (or GST is not set up).", color = Vui.TextTertiary, fontSize = 13.sp)
            Spacer(Modifier.height(16.dp))
            SectionTitle("Settlements")
            settlements.take(12).forEach { s ->
                GlassCard(Modifier.padding(bottom = 8.dp).fillMaxWidth()) {
                    Text(s.provider, color = Vui.TextPrimary, fontWeight = FontWeight.Bold)
                    Text(s.settledOn ?: s.id, color = Vui.TextTertiary, fontSize = 12.sp)
                    AmountText(Money(s.netMinor), 20)
                    Text("Gross ${Money(s.grossMinor).display} · fees ${Money(s.feeMinor).display}", color = Vui.TextSecondary, fontSize = 12.sp)
                }
            }
        }
    }
}

@Composable
fun AutoPayListScreen(
    mandates: List<MandateInfo>,
    message: String?,
    onRefresh: () -> Unit,
    onPause: (MandateInfo) -> Unit,
    onResume: (MandateInfo) -> Unit,
    onRevoke: (MandateInfo) -> Unit,
    onBack: (() -> Unit)? = null,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Merchant)
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                onBack?.let { TextButton(onClick = it) { Text("Back", color = Vui.Emerald) } }
                Text("AutoPay", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
                Spacer(Modifier.weight(1f))
                TestChip()
            }
            TextButton(onClick = onRefresh) { Text("Refresh", color = Vui.Emerald) }
            message?.let { Text(it, color = Vui.Amber, fontSize = 13.sp) }
            if (mandates.isEmpty()) {
                Text("No mandates yet. Create them in the console or from a payer’s Scan of a mandate QR.", color = Vui.TextTertiary)
            }
            mandates.forEach { m ->
                GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                    Text(m.id, color = Vui.TextPrimary, fontWeight = FontWeight.SemiBold)
                    Text(m.status, color = Vui.TextSecondary)
                    m.maxAmountMinor?.let { AmountText(Money(it), 18) }
                    m.frequency?.let { Text(it, color = Vui.TextTertiary, fontSize = 12.sp) }
                    Row {
                        if (m.status == "active") TextButton(onClick = { onPause(m) }) { Text("Pause", color = Vui.Amber) }
                        if (m.status == "paused") TextButton(onClick = { onResume(m) }) { Text("Resume", color = Vui.Emerald) }
                        TextButton(onClick = { onRevoke(m) }) { Text("Revoke", color = Vui.Coral) }
                    }
                }
            }
        }
    }
}

@Composable
fun MandateApproveScreen(
    name: String,
    payee: String,
    cap: Money,
    frequency: String,
    busy: Boolean,
    error: String?,
    onApprove: () -> Unit,
    onDecline: () -> Unit,
    onCancel: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().padding(20.dp)) {
            Text("AutoPay request", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(12.dp))
            GlassCard(Modifier.fillMaxWidth()) {
                Text(name.ifBlank { payee }, color = Vui.TextPrimary, fontWeight = FontWeight.Bold, fontSize = 20.sp)
                Text(payee, color = Vui.TextSecondary)
                Spacer(Modifier.height(8.dp))
                Text("Up to", color = Vui.TextTertiary, fontSize = 12.sp)
                AmountText(cap, 36)
                Text(frequency, color = Vui.TextSecondary)
            }
            error?.let { Text(it, color = Vui.Coral, modifier = Modifier.padding(top = 8.dp)) }
            Spacer(Modifier.weight(1f))
            Button(
                onClick = onApprove,
                enabled = !busy,
                modifier = Modifier.fillMaxWidth().height(52.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Vui.Indigo),
            ) { Text(if (busy) "…" else "Approve (test)") }
            TextButton(onClick = onDecline, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
                Text("Decline", color = Vui.Coral)
            }
            TextButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) { Text("Cancel", color = Vui.TextSecondary) }
        }
    }
}

@Composable
fun JarsScreen(
    jars: List<dev.zyvor.vayora.kit.Jar>,
    wallet: Money,
    onAdd: () -> Unit,
    onDeposit: (dev.zyvor.vayora.kit.Jar) -> Unit,
    onWithdraw: (dev.zyvor.vayora.kit.Jar) -> Unit,
    onRemove: (dev.zyvor.vayora.kit.Jar) -> Unit,
    onBack: () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        AmbientBackground(Brand.Pay)
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onBack) { Text("Back", color = Vui.Indigo) }
                Text("Jars", color = Vui.TextPrimary, fontSize = 28.sp, fontWeight = FontWeight.Bold)
            }
            Text("Spendable ${wallet.display}", color = Vui.TextSecondary)
            TextButton(onClick = onAdd) { Text("Add a jar", color = Vui.Indigo) }
            jars.forEach { j ->
                GlassCard(Modifier.padding(bottom = 10.dp).fillMaxWidth()) {
                    Text(j.name, color = Vui.TextPrimary, fontWeight = FontWeight.Bold)
                    Text("${Money(j.savedMinor).display} of ${Money(j.targetMinor).display}", color = Vui.TextSecondary)
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .height(8.dp)
                            .background(Vui.Hairline)
                            .padding(0.dp),
                    ) {
                        Box(
                            Modifier
                                .fillMaxWidth(j.fraction.toFloat())
                                .height(8.dp)
                                .background(Vui.Indigo),
                        )
                    }
                    Row {
                        TextButton(onClick = { onDeposit(j) }) { Text("Add ₹100", color = Vui.Emerald) }
                        TextButton(onClick = { onWithdraw(j) }) { Text("Take ₹100", color = Vui.Amber) }
                        TextButton(onClick = { onRemove(j) }) { Text("Delete", color = Vui.Coral) }
                    }
                }
            }
        }
    }
}

@Composable
private fun SectionTitle(t: String) {
    Text(t, color = Vui.TextPrimary, fontSize = 18.sp, fontWeight = FontWeight.Bold, modifier = Modifier.padding(vertical = 10.dp))
}

@Composable
private fun Stat(label: String, minor: Long) {
    Row(Modifier.fillMaxWidth().padding(vertical = 2.dp), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, color = Vui.TextSecondary, fontSize = 13.sp)
        Text(Money(minor).display, color = Vui.TextPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
    }
}
