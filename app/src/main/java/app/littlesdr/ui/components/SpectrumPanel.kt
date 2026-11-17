package app.littlesdr.ui.components

import androidx.compose.foundation.AndroidExternalSurface
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import app.littlesdr.engine.RdsInfo
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.material3.Text
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.basicMarquee
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.fadeOut
import androidx.compose.animation.fadeIn
import androidx.compose.animation.expandVertically
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.ui.Alignment
import androidx.compose.material3.Surface
import androidx.compose.material3.IconButton
import androidx.compose.material3.Icon
import androidx.compose.material.icons.filled.ZoomOutMap
import androidx.compose.material.icons.filled.ZoomOut
import androidx.compose.material.icons.filled.ZoomIn
import androidx.compose.material.icons.Icons
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.littlesdr.data.DisplaySettings
import app.littlesdr.engine.Mode
import app.littlesdr.engine.NativeBridge
import app.littlesdr.engine.RadioState
import app.littlesdr.ui.ViewState
import app.littlesdr.ui.axisLabel
import app.littlesdr.ui.niceStep
import app.littlesdr.ui.snap
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor

/**
 * Spectrum trace + waterfall. Pixels come from the native Vulkan renderer; this
 * composable only adds the overlay (scale, VFO, squelch) and the gestures.
 */
@Composable
fun SpectrumPanel(
    // State holders (not values): read only in gesture handlers and in the draw phase,
    // so tuning redraws the overlay without recomposing anything.
    state: State<RadioState>,
    view: State<ViewState>,
    display: DisplaySettings,
    step: Double,
    onTune: (Double) -> Unit,
    onView: (bandwidth: Double, offset: Double) -> Unit,
    onResetZoom: () -> Unit,
    modifier: Modifier = Modifier,
    cornerRadius: Dp = 0.dp,
    maskColor: Color = Color.Transparent,
    rds: RdsInfo = RdsInfo(),
) {
    val st by state
    val vw by view
    val stepHz by rememberUpdatedState(step)
    val tune by rememberUpdatedState(onTune)
    val setView by rememberUpdatedState(onView)
    val touchSlop = LocalViewConfiguration.current.touchSlop

    val primary = MaterialTheme.colorScheme.primary
    val secondary = MaterialTheme.colorScheme.secondary
    val labelColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.75f)
    val gridColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
    val measurer = rememberTextMeasurer(cacheSize = 32) // ~13 labels per frame
    val labelStyle = TextStyle(fontSize = 10.sp, color = labelColor, fontFeatureSettings = "tnum")

    Box(modifier) {
        AndroidExternalSurface(Modifier.fillMaxSize()) {
            onSurface { surface, _, _ ->
                NativeBridge.surfaceAttach(surface)
                surface.onChanged { w, h -> NativeBridge.surfaceResize(w, h) }
                surface.onDestroyed { NativeBridge.surfaceDetach() }
            }
        }

        Canvas(
            Modifier
                .fillMaxSize()
                .graphicsLayer() // overlay redraws on every tune; keep it isolated
                .pointerInput(Unit) {
                    // No tap-to-tune (tuning is the joystick's job): double tap zooms ×2 on the spot
                    detectTapGestures(
                        onDoubleTap = { p ->
                            val bw = vw.bandwidth.takeIf { it > 0 } ?: st.sampleRate
                            val u = p.x / size.width - 0.5
                            val focus = vw.offset + u * bw
                            val newBw = bw / 2
                            setView(newBw, focus - u * newBw)
                        },
                    )
                }
                .pointerInput(Unit) {
                    awaitEachGesture {
                        awaitFirstDown(requireUnconsumed = false)
                        var dragging = false
                        var totalDx = 0f
                        do {
                            val event = awaitPointerEvent()
                            val pressed = event.changes.count { it.pressed }
                            val w = size.width.toFloat()
                            if (pressed >= 2) {
                                // Pinch: zoom around the fingers, two-finger pan moves the view
                                val zoom = event.calculateZoom()
                                val pan = event.calculatePan()
                                val c = event.calculateCentroid(useCurrent = true)
                                val bw = vw.bandwidth.takeIf { it > 0 } ?: st.sampleRate
                                val newBw = bw / zoom
                                val u = c.x / w - 0.5
                                val focus = vw.offset + u * bw
                                setView(newBw, focus - u * newBw - pan.x * newBw / w)
                                dragging = true
                                event.changes.forEach { it.consume() }
                            } else if (pressed == 1) {
                                // One finger pans the view (when zoomed in); tuning is done with
                                // a tap or the joystick, so brushing the spectrum never detunes
                                val ch = event.changes.first { it.pressed }
                                val dx = ch.positionChange().x
                                totalDx += dx
                                if (!dragging && abs(totalDx) > touchSlop) dragging = true
                                if (dragging) {
                                    val bw = vw.bandwidth.takeIf { it > 0 } ?: st.sampleRate
                                    setView(bw, vw.offset - dx * bw / w)
                                    ch.consume()
                                }
                            }
                        } while (event.changes.any { it.pressed })
                    }
                },
        ) {
            val w = size.width
            val h = size.height
            val specH = floor(h * display.spectrumFraction.coerceIn(0.1f, 0.9f))
            val state = st
            val view = vw
            val sr = state.sampleRate
            if (sr <= 0) return@Canvas
            val bw = view.bandwidth.takeIf { it > 0 } ?: sr
            val left = state.centerFrequency + view.offset - bw / 2
            fun xOf(f: Double) = ((f - left) / bw * w).toFloat()

            // dB grid (every 10 dB) with labels on the left
            val dbSpan = (display.dbMax - display.dbMin).coerceAtLeast(1f)
            var db = ceil(display.dbMin / 10f) * 10f
            while (db <= display.dbMax) {
                val y = specH * (1f - (db - display.dbMin) / dbSpan)
                drawLine(gridColor, Offset(0f, y), Offset(w, y), 1f)
                // No label inside the rounded top-left corner
                if (y > maxOf(12.dp.toPx(), cornerRadius.toPx() * 0.8f) && y < specH - 14.dp.toPx()) {
                    drawText(measurer, "${db.toInt()}", Offset(4.dp.toPx(), y - 13.dp.toPx()), labelStyle)
                }
                db += 10f
            }

            // Frequency scale along the spectrum/waterfall boundary
            val tickStep = niceStep(bw, (w / 90.dp.toPx()).toInt().coerceAtLeast(2))
            var f = ceil(left / tickStep) * tickStep
            while (f < left + bw) {
                val x = xOf(f)
                drawLine(gridColor, Offset(x, 0f), Offset(x, specH), 1f)
                drawLine(labelColor, Offset(x, specH - 5.dp.toPx()), Offset(x, specH), 1.dp.toPx())
                val text = axisLabel(f, tickStep)
                val layout = measurer.measure(text, labelStyle)
                val tx = (x - layout.size.width / 2f).coerceIn(0f, w - layout.size.width)
                drawText(layout, topLeft = Offset(tx, specH - 7.dp.toPx() - layout.size.height))
                f += tickStep
            }

            // VFO passband + tuned frequency
            if (state.vfoBandwidth > 0) {
                val tuned = state.centerFrequency + state.vfoOffset
                val vbw = state.vfoBandwidth
                val (lo, hi) = when (state.mode) {
                    Mode.USB -> tuned to tuned + vbw
                    Mode.LSB -> tuned - vbw to tuned
                    else -> tuned - vbw / 2 to tuned + vbw / 2
                }
                val x0 = xOf(lo)
                val x1 = xOf(hi).coerceAtLeast(x0 + 2f)
                drawRect(primary.copy(alpha = 0.18f), Offset(x0, 0f), Size(x1 - x0, specH))
                drawRect(primary.copy(alpha = 0.07f), Offset(x0, specH), Size(x1 - x0, h - specH))
                val xt = xOf(tuned)
                drawLine(primary, Offset(xt, 0f), Offset(xt, specH), 1.5.dp.toPx())
                drawLine(primary.copy(alpha = 0.45f), Offset(xt, specH), Offset(xt, h), 1.dp.toPx())
            }

            // Squelch threshold
            if (state.squelchEnabled) {
                val y = specH * (1f - (state.squelchLevel - display.dbMin) / dbSpan)
                drawLine(
                    secondary, Offset(0f, y), Offset(w, y), 1.dp.toPx(),
                    pathEffect = PathEffect.dashPathEffect(floatArrayOf(8.dp.toPx(), 6.dp.toPx())),
                )
            }

            drawLine(Color.Black.copy(alpha = 0.4f), Offset(0f, specH), Offset(w, specH), 1.dp.toPx())

        }

        // RDS notch hanging from the top edge: station name, programme type, scrolling RadioText
        AnimatedVisibility(
            visible = rds.hasData,
            enter = fadeIn() + expandVertically(expandFrom = Alignment.Top),
            exit = fadeOut() + shrinkVertically(shrinkTowards = Alignment.Top),
            modifier = Modifier.align(Alignment.TopCenter),
        ) {
            RdsNotch(rds, Modifier.fillMaxWidth(0.5f))
        }

        ZoomButtons(
            onZoomIn = { zoomAroundVfo(st, vw, 0.5, setView) },
            onZoomOut = { zoomAroundVfo(st, vw, 2.0, setView) },
            onReset = onResetZoom,
            modifier = Modifier.align(Alignment.TopEnd).padding(top = 10.dp, end = 10.dp),
        )

        // Rounded corners: the Vulkan SurfaceView can't be clipped by Compose,
        // so paint the outside of a rounded rect in the background color on top of it.
        if (cornerRadius > 0.dp) {
            Canvas(Modifier.fillMaxSize().graphicsLayer()) { // static: recorded once
                val r = cornerRadius.toPx()
                val mask = Path().apply {
                    fillType = PathFillType.EvenOdd
                    addRect(Rect(0f, 0f, size.width, size.height))
                    addRoundRect(RoundRect(0f, 0f, size.width, size.height, CornerRadius(r, r)))
                }
                drawPath(mask, maskColor)
            }
        }
    }
}

private fun freqAt(x: Float, width: Float, s: RadioState, v: ViewState): Double {
    val bw = v.bandwidth.takeIf { it > 0 } ?: s.sampleRate
    return s.centerFrequency + v.offset - bw / 2 + x / width * bw
}


/** Zoom by [factor] (0.5 = in) keeping the VFO where it is on screen when it's visible. */
private fun zoomAroundVfo(s: RadioState, v: ViewState, factor: Double, setView: (Double, Double) -> Unit) {
    val bw = v.bandwidth.takeIf { it > 0 } ?: s.sampleRate
    val newBw = bw * factor
    val vfo = s.vfoOffset
    val visible = vfo >= v.offset - bw / 2 && vfo <= v.offset + bw / 2
    val anchor = if (visible) vfo else v.offset
    val u = (anchor - v.offset) / bw // anchor position in the view, -0.5..0.5
    setView(newBw, anchor - u * newBw)
}

@Composable
private fun ZoomButtons(onZoomIn: () -> Unit, onZoomOut: () -> Unit, onReset: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.85f),
        tonalElevation = 2.dp,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            IconButton(onClick = onZoomIn, modifier = Modifier.size(40.dp)) { Icon(Icons.Default.ZoomIn, "Zoomer") }
            IconButton(onClick = onZoomOut, modifier = Modifier.size(40.dp)) { Icon(Icons.Default.ZoomOut, "Dézoomer") }
            IconButton(onClick = onReset, modifier = Modifier.size(40.dp)) { Icon(Icons.Default.ZoomOutMap, "Toute la bande") }
        }
    }
}


@Composable
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
private fun RdsNotch(rds: RdsInfo, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(bottomStart = 20.dp, bottomEnd = 20.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.92f),
        tonalElevation = 3.dp,
    ) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 6.dp, bottom = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    rds.ps.ifBlank { rds.callsign.ifBlank { "RDS" } },
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.primary,
                    maxLines = 1,
                )
                if (rds.pty.isNotBlank()) {
                    Text(
                        "  ·  ${rds.pty}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                    )
                }
            }
            if (rds.rt.isNotBlank()) {
                Text(
                    rds.rt,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    modifier = Modifier.fillMaxWidth().basicMarquee(iterations = Int.MAX_VALUE),
                )
            }
        }
    }
}
