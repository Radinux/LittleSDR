package app.littlesdr.ui.components

import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.abs
import kotlin.math.pow

private const val DIGITS = 10 // up to 9.999 999 999 GHz

/**
 * Big frequency readout (Hz, grouped by 3). Each digit is a control:
 * drag up/down to change it continuously, tap its upper/lower half for +1/-1.
 * Leading zeros are dimmed, like a real receiver's display.
 */
@Composable
fun FrequencyDisplay(
    frequencyHz: Double,
    onTune: (Double) -> Unit,
    modifier: Modifier = Modifier,
) {
    val freq by rememberUpdatedState(frequencyHz)
    val tune by rememberUpdatedState(onTune)
    val haptics = LocalHapticFeedback.current
    val digits = frequencyHz.toLong().coerceAtLeast(0).toString().padStart(DIGITS, '0').takeLast(DIGITS)
    val firstSignificant = digits.indexOfFirst { it != '0' }.let { if (it < 0) DIGITS - 1 else it }

    BoxWithConstraints(modifier) {
        // 10 digits + 3 separators, sized to the available width
        val fontSize = with(LocalDensity.current) { (maxWidth / 8.2f).toSp() }.value.coerceAtMost(54f).sp
        Row(verticalAlignment = Alignment.Bottom) {
            digits.forEachIndexed { i, ch ->
                val place = 10.0.pow(DIGITS - 1 - i)
                val dim = i < firstSignificant
                val color = if (dim) MaterialTheme.colorScheme.onSurface.copy(alpha = 0.22f) else MaterialTheme.colorScheme.onSurface
                Text(
                    text = ch.toString(),
                    color = color,
                    fontSize = fontSize,
                    fontWeight = FontWeight.Light,
                    style = MaterialTheme.typography.displayMedium,
                    modifier = Modifier
                        .pointerInput(place) {
                            detectTapGestures { pos ->
                                val dir = if (pos.y < size.height / 2) 1 else -1
                                haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                tune((freq + dir * place).coerceAtLeast(0.0))
                            }
                        }
                        .pointerInput(place) {
                            var acc = 0f
                            val stepPx = 18.dp.toPx()
                            detectVerticalDragGestures(onDragStart = { acc = 0f }) { change, dy ->
                                change.consume()
                                acc -= dy
                                if (abs(acc) >= stepPx) {
                                    val n = (acc / stepPx).toInt()
                                    acc -= n * stepPx
                                    haptics.performHapticFeedback(HapticFeedbackType.SegmentFrequentTick)
                                    tune((freq + n * place).coerceAtLeast(0.0))
                                }
                            }
                        },
                )
                val sepAfter = DIGITS - 1 - i
                if (sepAfter > 0 && sepAfter % 3 == 0) {
                    Text(
                        ".",
                        color = if (i < firstSignificant - 1) color else MaterialTheme.colorScheme.primary,
                        fontSize = fontSize * 0.7f,
                        modifier = Modifier.padding(horizontal = 1.dp),
                    )
                }
            }
        }
    }
}
