package com.obsync.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

// Semantic status colors, theme-aware so the dots adapt to dark mode.
// Exposed via CompositionLocal so screens read `LocalStatusColors.current`.
data class StatusColors(
    val idle: Color,    // green
    val syncing: Color, // amber
    val offline: Color, // gray
    val conflict: Color, // red
)

val LocalStatusColors = staticCompositionLocalOf { lightStatusColors }

private val lightStatusColors = StatusColors(
    idle = Color(0xFF16A34A),
    syncing = Color(0xFFD97706),
    offline = Color(0xFF737373),
    conflict = Color(0xFFDC2626),
)

private val darkStatusColors = StatusColors(
    idle = Color(0xFF4ADE80),
    syncing = Color(0xFFFBBF24),
    offline = Color(0xFF9CA3AF),
    conflict = Color(0xFFF87171),
)

// Explicit type scale so screens never hard-code sizes:
// titleMedium 16sp, titleSmall 14sp, bodyMedium 14sp, bodySmall 12sp, labelMedium 12sp.
private val AppTypography = Typography(
    titleMedium = TextStyle(fontSize = 16.sp, lineHeight = 24.sp, fontWeight = FontWeight.Medium),
    titleSmall = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Normal),
    bodySmall = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Normal),
    labelMedium = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Medium),
)

private val Light = lightColorScheme(
    primary = Color(0xFF2563EB),
    onPrimary = Color.White,
    surface = Color(0xFFFFFFFF),
    background = Color(0xFFFAFAFA),
    onSurface = Color(0xFF171717),
    onSurfaceVariant = Color(0xFF737373),
    error = Color(0xFFDC2626),
)

private val Dark = darkColorScheme(
    primary = Color(0xFF3B82F6),
    onPrimary = Color.White,
    surface = Color(0xFF1A1A1A),
    background = Color(0xFF0A0A0A),
    onSurface = Color(0xFFF5F5F5),
    onSurfaceVariant = Color(0xFFA3A3A3),
    error = Color(0xFFDC2626),
)

@Composable
fun ObsyncTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit
) {
    val statusColors = if (darkTheme) darkStatusColors else lightStatusColors
    CompositionLocalProvider(LocalStatusColors provides statusColors) {
        MaterialTheme(
            colorScheme = if (darkTheme) Dark else Light,
            typography = AppTypography,
            content = content,
        )
    }
}
