package app.littlesdr.ui.components

import androidx.compose.foundation.border
import app.littlesdr.ui.theme.pillOutline
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardDoubleArrowLeft
import androidx.compose.material.icons.filled.KeyboardDoubleArrowRight
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import app.littlesdr.ui.snap
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.pow
import kotlin.math.sign

private const val DEAD_ZONE = 0.04f

/**
 * Linear "rate" joystick: the further the thumb is from the center, the faster the VFO moves.
 * Releasing springs the thumb back and stops tuning. Cubic response: fine steps near the
 * center, about 1.5 visible spans per second at full deflection.
 */
@Composable
fun TuningJoystick(
    frequency: () -> Double,
    span: () -> Double,
    step: () -> Double,
    onTune: (Double) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tune by rememberUpdatedState(onTune)
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    val deflection = remember { Animatable(0f) }
    var active by remember { mutableStateOf(false) }

    fun rateFor(d: Float): Double {
        val a = abs(d)
        if (a < DEAD_ZONE) return 0.0
        val s = step()
        val slow = s * 2.0
        val fast = (span() * 1.5).coerceAtLeast(slow)
        return sign(d) * (slow + (fast - slow) * ((a - DEAD_ZONE) / (1f - DEAD_ZONE)).pow(3))
    }

    // Tuning loop: only runs (and requests frames) while the finger is down
    LaunchedEffect(active) {
        if (!active) return@LaunchedEffect
        var target = frequency()
        var lastSent = target
        var last = withFrameNanos { it }
        while (active) {
            val now = withFrameNanos { it }
            val dt = ((now - last) / 1e9).coerceIn(0.0, 0.1)
            last = now
            val r = rateFor(deflection.value)
            if (r == 0.0) continue
            target += r * dt
            val snapped = snap(target, step())
            if (snapped != lastSent) {
                lastSent = snapped
                tune(snapped)
                if (abs(r) < step() * 15) haptics.performHapticFeedback(HapticFeedbackType.SegmentFrequentTick)
            }
        }
    }

    val primary = MaterialTheme.colorScheme.primary
    val onPrimary = MaterialTheme.colorScheme.onPrimary
    val track = MaterialTheme.colorScheme.surfaceContainerHigh
    val onTrack = MaterialTheme.colorScheme.onSurfaceVariant

    Box(
        modifier
            .clip(RoundedCornerShape(50))
            .background(track)
            .border(1.dp, pillOutline(), RoundedCornerShape(50))
            .pointerInput(Unit) {
                awaitEachGesture {
                    val down = awaitFirstDown()
                    val half = size.width / 2f
                    fun deflect(x: Float) = ((x - half) / half).coerceIn(-1f, 1f)
                    active = true
                    haptics.performHapticFeedback(HapticFeedbackType.GestureThresholdActivate)
                    scope.launch { deflection.snapTo(deflect(down.position.x)) }
                    while (true) {
                        val ev = awaitPointerEvent()
                        val ch = ev.changes.firstOrNull { it.id == down.id } ?: break
                        if (!ch.pressed) break
                        ch.consume()
                        scope.launch { deflection.snapTo(deflect(ch.position.x)) }
                    }
                    active = false
                    scope.launch {
                        deflection.animateTo(0f, spring(dampingRatio = Spring.DampingRatioMediumBouncy, stiffness = Spring.StiffnessMedium))
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Default.KeyboardDoubleArrowLeft, null, tint = onTrack.copy(alpha = 0.5f),
            modifier = Modifier.align(Alignment.CenterStart).padding(start = 12.dp))
        Icon(Icons.Default.KeyboardDoubleArrowRight, null, tint = onTrack.copy(alpha = 0.5f),
            modifier = Modifier.align(Alignment.CenterEnd).padding(end = 12.dp))

        Canvas(Modifier.fillMaxSize()) {
            val cx = size.width / 2f
            val thumbW = 72.dp.toPx()
            val thumbH = size.height - 10.dp.toPx()
            val x = cx + deflection.value * (cx - thumbW / 2f - 4.dp.toPx())
            // Fill between center and thumb shows direction and amount
            val l = minOf(cx, x)
            drawRoundRect(primary.copy(alpha = 0.18f), Offset(l, 5.dp.toPx()), Size(abs(x - cx), thumbH), CornerRadius(thumbH / 2))
            drawLine(onTrack.copy(alpha = 0.35f), Offset(cx, size.height * 0.3f), Offset(cx, size.height * 0.7f), 2.dp.toPx())
            drawRoundRect(primary, Offset(x - thumbW / 2f, 5.dp.toPx()), Size(thumbW, thumbH), CornerRadius(thumbH / 2))
            // Grip: two short bars
            val gx = x
            val gy = size.height / 2f
            val gh = thumbH * 0.32f
            for (dx in floatArrayOf(-4.dp.toPx(), 4.dp.toPx())) {
                drawLine(onPrimary.copy(alpha = 0.7f), Offset(gx + dx, gy - gh / 2), Offset(gx + dx, gy + gh / 2), 2.dp.toPx(),
                    cap = androidx.compose.ui.graphics.StrokeCap.Round)
            }
        }
    }
}
