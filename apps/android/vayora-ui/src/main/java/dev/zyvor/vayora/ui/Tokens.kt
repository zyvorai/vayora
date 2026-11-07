package dev.zyvor.vayora.ui

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/** Design tokens mirrored from iOS `VUI` / `Brand`. Dark-first canvas. */
object Vui {
    val Ink = Color(0xFF07070B)
    val InkRaised = Color(0xFF13131B)
    val Hairline = Color.White.copy(alpha = 0.10f)
    val TextPrimary = Color.White
    val TextSecondary = Color.White.copy(alpha = 0.62f)
    val TextTertiary = Color.White.copy(alpha = 0.38f)

    val Indigo = Color(0xFF6B57FF)
    val IndigoDeep = Color(0xFF291A8C)
    val Violet = Color(0xFFB861FF)
    val Emerald = Color(0xFF1AD194)
    val EmeraldDeep = Color(0xFF055C45)
    val Mint = Color(0xFF85FFD1)
    val Amber = Color(0xFFFFB833)
    val Coral = Color(0xFFFF5766)

    val SpaceS = 8.dp
    val SpaceM = 16.dp
    val SpaceL = 24.dp
    val RadiusCard = 28.dp
    val RadiusControl = 20.dp
}

data class Brand(val primary: Color, val deep: Color, val glow: Color) {
    val gradient: Brush
        get() = Brush.linearGradient(listOf(primary, deep))

    companion object {
        val Pay = Brand(Vui.Indigo, Vui.IndigoDeep, Vui.Violet)
        val Merchant = Brand(Vui.Emerald, Vui.EmeraldDeep, Vui.Mint)
    }
}
