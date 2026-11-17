package app.littlesdr.ui

import android.os.Build
import android.view.RoundedCorner
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.max

/** Geometry derived from the display's rounded corners. */
data class ScreenCorners(
    /** Margin keeping content clear of the rounded corners. */
    val margin: Dp,
    /** Radius for our own containers so their corners stay concentric with the screen's. */
    val innerRadius: Dp,
)

/**
 * Reads the real corner radius (Android 12+). A margin of ~0.15·r (half the 0.3·r a corner cuts
 * into the diagonal) keeps content clear while staying tight; inner corners stay concentric.
 */
@Composable
fun rememberScreenCorners(): ScreenCorners {
    val view = LocalView.current
    val density = LocalDensity.current
    return remember(view, density) {
        val radiusPx = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val insets = view.rootWindowInsets
            listOf(
                RoundedCorner.POSITION_TOP_LEFT, RoundedCorner.POSITION_TOP_RIGHT,
                RoundedCorner.POSITION_BOTTOM_LEFT, RoundedCorner.POSITION_BOTTOM_RIGHT,
            ).maxOfOrNull { insets?.getRoundedCorner(it)?.radius ?: 0 } ?: 0
        } else 0
        val radius = with(density) { radiusPx.toDp() }
        val margin = max(radius * 0.15f + 2.dp, 5.dp)
        ScreenCorners(margin = margin, innerRadius = max(radius - margin, 16.dp))
    }
}
