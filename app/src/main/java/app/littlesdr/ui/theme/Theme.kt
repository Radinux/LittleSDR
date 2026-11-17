package app.littlesdr.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import app.littlesdr.data.Appearance

/** Accent colors used when Material You (wallpaper colors) is off. Tones ~80/20/30/90 (dark), ~40 (light). */
enum class Accent(
    val label: String,
    val primary: Long, val onPrimary: Long, val container: Long, val onContainer: Long,
    val lightPrimary: Long,
) {
    Indigo("Indigo", 0xFFB8C4FF, 0xFF1D2D61, 0xFF344479, 0xFFDDE1FF, 0xFF4B5C92),
    Amber("Ambre", 0xFFFFB86E, 0xFF4A2800, 0xFF6A3C00, 0xFFFFDCBE, 0xFF8B5000),
    Mint("Menthe", 0xFF7DD8C6, 0xFF00382F, 0xFF005045, 0xFF9AF4E1, 0xFF006B5D),
    Rose("Rose", 0xFFFFB0CB, 0xFF5E1133, 0xFF7B2949, 0xFFFFD9E3, 0xFF99405E),
    Sky("Ciel", 0xFF9ECAFF, 0xFF003258, 0xFF00497D, 0xFFD1E4FF, 0xFF0061A4),
    Lime("Lime", 0xFFBFD66F, 0xFF2B3400, 0xFF3F4C00, 0xFFDBF288, 0xFF556500);

    companion object {
        fun of(i: Int) = entries.getOrElse(i) { Indigo }
    }
}

private val DarkBase = darkColorScheme(
    secondary = Color(0xFFC4C6D0),
    tertiary = Color(0xFF7DD8C6),
    background = Color(0xFF111318),
    surface = Color(0xFF111318),
    surfaceContainerLowest = Color(0xFF0C0E13),
    surfaceContainerLow = Color(0xFF191B21),
    surfaceContainer = Color(0xFF1D2025),
    surfaceContainerHigh = Color(0xFF282A30),
    surfaceContainerHighest = Color(0xFF33353B),
)

private fun accentScheme(a: Accent, dark: Boolean): ColorScheme =
    if (dark) {
        DarkBase.copy(
            primary = Color(a.primary), onPrimary = Color(a.onPrimary),
            primaryContainer = Color(a.container), onPrimaryContainer = Color(a.onContainer),
            secondaryContainer = Color(a.container), onSecondaryContainer = Color(a.onContainer),
            surfaceTint = Color(a.primary),
        )
    } else {
        val p = Color(a.lightPrimary)
        val container = lerp(p, Color.White, 0.82f)
        lightColorScheme(
            primary = p, onPrimary = Color.White,
            primaryContainer = container, onPrimaryContainer = lerp(p, Color.Black, 0.55f),
            secondaryContainer = container, onSecondaryContainer = lerp(p, Color.Black, 0.55f),
            surfaceTint = p,
        )
    }

private val AppTypography = Typography().let { t ->
    t.copy(
        displayMedium = t.displayMedium.copy(fontWeight = FontWeight.Light, fontFeatureSettings = "tnum"),
        labelSmall = t.labelSmall.copy(fontFeatureSettings = "tnum"),
    )
}

/** Monospaced-digit style for frequencies and readouts. */
val ReadoutStyle = TextStyle(fontFeatureSettings = "tnum", fontWeight = FontWeight.Medium, fontSize = 14.sp)

val dynamicColorAvailable: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

@Composable
fun LittleSdrTheme(appearance: Appearance = Appearance(), content: @Composable () -> Unit) {
    val ctx = LocalContext.current
    val dark = when (appearance.themeMode) {
        0 -> isSystemInDarkTheme()
        2 -> false
        else -> true
    }
    val scheme = if (appearance.dynamicColor && dynamicColorAvailable) {
        if (dark) dynamicDarkColorScheme(ctx) else dynamicLightColorScheme(ctx)
    } else {
        accentScheme(Accent.of(appearance.accent), dark)
    }
    MaterialTheme(colorScheme = scheme, typography = AppTypography, content = content)
}
