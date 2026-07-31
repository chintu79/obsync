package com.obsync.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

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
    MaterialTheme(
        colorScheme = if (darkTheme) Dark else Light,
        content = content,
    )
}
