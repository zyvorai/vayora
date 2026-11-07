package dev.zyvor.vayora.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Science
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.zyvor.vayora.kit.Money

@Composable
fun AmbientBackground(brand: Brand, modifier: Modifier = Modifier) {
    Box(
        modifier
            .fillMaxSize()
            .background(Vui.Ink)
            .background(
                Brush.radialGradient(
                    colors = listOf(brand.primary.copy(alpha = 0.28f), Color.Transparent),
                    radius = 900f,
                ),
            )
            .background(
                Brush.radialGradient(
                    colors = listOf(brand.glow.copy(alpha = 0.12f), Color.Transparent),
                    center = androidx.compose.ui.geometry.Offset(200f, 1400f),
                    radius = 700f,
                ),
            ),
    )
}

@Composable
fun GlassCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier
            .clip(RoundedCornerShape(Vui.RadiusCard))
            .background(Color.White.copy(alpha = 0.07f))
            .border(0.5.dp, Vui.Hairline, RoundedCornerShape(Vui.RadiusCard))
            .padding(18.dp),
        content = content,
    )
}

@Composable
fun TestChip(modifier: Modifier = Modifier) {
    val pill = RoundedCornerShape(999.dp)
    Row(
        modifier
            .clip(pill)
            .background(Vui.Amber.copy(alpha = 0.14f))
            .border(0.5.dp, Vui.Amber.copy(alpha = 0.3f), pill)
            .padding(horizontal = 10.dp, vertical = 5.dp)
            .semantics { contentDescription = "Test mode. No real money moves." },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Filled.Science, null, tint = Vui.Amber, modifier = Modifier.size(14.dp).padding(end = 2.dp))
        Text("Test mode", color = Vui.Amber, fontSize = 11.5.sp, fontWeight = FontWeight.Bold)
    }
}

@Composable
fun AmountText(money: Money, size: Int = 44, modifier: Modifier = Modifier) {
    Text(
        money.display,
        modifier = modifier,
        color = Vui.TextPrimary,
        fontSize = size.sp,
        fontWeight = FontWeight.Bold,
        letterSpacing = (-0.5).sp,
    )
}
