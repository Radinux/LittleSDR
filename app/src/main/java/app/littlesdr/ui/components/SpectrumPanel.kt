package app.littlesdr.ui.components

import app.littlesdr.ui.theme.pillOutline
import app.littlesdr.engine.BandPlan
import app.littlesdr.engine.BandKind
import androidx.compose.foundation.AndroidExternalSurface
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.Spacer
import app.littlesdr.data.EiBiEntry
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
    eibi: List<EiBiEntry> = emptyList(),
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
    val bandLabelStyle = TextStyle(fontSize = 9.sp, color = Color.White, fontWeight = FontWeight.Medium)

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
                // Passband clipped to the view. When it covers most of the view (narrow sources
                // like KiwiSDR with a wide mode), don't fill it: that would wash out the whole display.
                val x0 = xOf(lo).coerceIn(0f, w)
                val x1 = xOf(hi).coerceIn(0f, w).coerceAtLeast(x0 + 2f)
                if (x1 - x0 < w * 0.85f) {
                    drawRect(primary.copy(alpha = 0.18f), Offset(x0, 0f), Size(x1 - x0, specH))
                    drawRect(primary.copy(alpha = 0.07f), Offset(x0, specH), Size(x1 - x0, h - specH))
                } else {
                    // Edge ticks only
                    val tick = 10.dp.toPx()
                    drawLine(primary.copy(alpha = 0.6f), Offset(x0 + 1f, 0f), Offset(x0 + 1f, tick), 2.dp.toPx())
                    drawLine(primary.copy(alpha = 0.6f), Offset(x1 - 1f, 0f), Offset(x1 - 1f, tick), 2.dp.toPx())
                }
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

            // Band plan strip along the bottom of the waterfall
            if (display.bandPlan) {
                val stripH = BAND_STRIP_HEIGHT.toPx()
                val y0 = h - stripH
                drawRect(Color.Black.copy(alpha = 0.35f), Offset(0f, y0), Size(w, stripH))
                BandPlan.visible(display.ituRegion, left, left + bw).forEach { seg ->
                    val x0 = xOf(seg.startHz).coerceIn(0f, w)
                    val x1 = xOf(seg.endHz).coerceIn(0f, w).coerceAtLeast(x0 + 3f)
                    val c = seg.kind.color
                    drawRect(c.copy(alpha = 0.55f), Offset(x0, y0), Size(x1 - x0, stripH))
                    drawLine(c, Offset(x0, y0), Offset(x1, y0), 2.dp.toPx())
                    // Type of allocation (HAM, Broadcast, Avion…), not the band's wavelength
                    val layout = measurer.measure(seg.tag, bandLabelStyle)
                    if (layout.size.width + 8.dp.toPx() < x1 - x0) {
                        // Label centered on the visible part of the band
                        val tx = ((x0 + x1) / 2f - layout.size.width / 2f)
                        drawText(layout, topLeft = Offset(tx, y0 + (stripH - layout.size.height) / 2f))
                    }
                }
            }

        }

        // RDS notch hanging from the top edge: station name, programme type, scrolling RadioText
        AnimatedVisibility(
            visible = rds.hasData || eibi.isNotEmpty(),
            enter = fadeIn() + expandVertically(expandFrom = Alignment.Top),
            exit = fadeOut() + shrinkVertically(shrinkTowards = Alignment.Top),
            modifier = Modifier.align(Alignment.TopCenter),
        ) {
            // Long and thin: a wide, low tab hanging from the top edge
            if (rds.hasData) RdsNotch(rds, Modifier.fillMaxWidth(0.62f))
            else if (eibi.isNotEmpty()) EiBiNotch(eibi, Modifier.fillMaxWidth(0.62f))
        }

        // Zoom is done with the fingers (pinch, double tap); one button brings the whole band
        // back, shown only while zoomed in
        val zoomed = vw.bandwidth > 0 && st.sampleRate > 0 && vw.bandwidth < st.sampleRate * 0.98
        AnimatedVisibility(
            visible = zoomed,
            enter = fadeIn(),
            exit = fadeOut(),
            modifier = Modifier.align(Alignment.BottomStart)
                .padding(start = 14.dp, bottom = if (display.bandPlan) BAND_STRIP_HEIGHT + 8.dp else 12.dp),
        ) {
            ResetZoomButton(onResetZoom)
        }

        // Rounded corners: the Vulkan SurfaceView can't be clipped by Compose,
        // so paint the outside of a rounded rect in the background color on top of it.
        if (cornerRadius > 0.dp) {
            val outline = pillOutline()
            Canvas(Modifier.fillMaxSize().graphicsLayer()) { // static: recorded once
                val r = cornerRadius.toPx()
                val mask = Path().apply {
                    fillType = PathFillType.EvenOdd
                    addRect(Rect(0f, 0f, size.width, size.height))
                    addRoundRect(RoundRect(0f, 0f, size.width, size.height, CornerRadius(r, r)))
                }
                drawPath(mask, maskColor)
                // The panel's outline, like the other pills
                val w = 1.dp.toPx()
                drawRoundRect(
                    outline,
                    topLeft = androidx.compose.ui.geometry.Offset(w / 2, w / 2),
                    size = androidx.compose.ui.geometry.Size(size.width - w, size.height - w),
                    cornerRadius = CornerRadius(r - w / 2, r - w / 2),
                    style = androidx.compose.ui.graphics.drawscope.Stroke(w),
                )
            }
        }
    }
}

private fun freqAt(x: Float, width: Float, s: RadioState, v: ViewState): Double {
    val bw = v.bandwidth.takeIf { it > 0 } ?: s.sampleRate
    return s.centerFrequency + v.offset - bw / 2 + x / width * bw
}


@Composable
private fun ResetZoomButton(onReset: () -> Unit) {
    Surface(
        onClick = onReset,
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.85f),
        tonalElevation = 2.dp,
        modifier = Modifier.size(44.dp),
    ) {
        Box(contentAlignment = Alignment.Center) { Icon(Icons.Default.ZoomOutMap, "Toute la bande") }
    }
}


@Composable
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
private fun RdsNotch(rds: RdsInfo, modifier: Modifier = Modifier) {
    NotchSurface(modifier) {
        NotchTitle(rds.ps.ifBlank { rds.callsign.ifBlank { "RDS" } }, rds.pty)
        if (rds.rt.isNotBlank()) NotchTicker(rds.rt)
    }
}

/** EiBi schedule for the tuned frequency: first station on top, the others scroll below. */
@Composable
private fun EiBiNotch(entries: List<EiBiEntry>, modifier: Modifier = Modifier) {
    val first = entries.first()
    NotchSurface(modifier) {
        NotchTitle(first.station, listOf(first.languageText, first.target).filter { it.isNotBlank() }.joinToString(" → "))
        NotchTicker(
            if (entries.size > 1) {
                entries.drop(1).joinToString("   ·   ") { e ->
                    listOf(e.station, e.languageText).filter { it.isNotBlank() }.joinToString(" ") + " (${e.timeText})"
                }
            } else {
                "${first.timeText} UTC · ${first.country} · EiBi"
            }
        )
    }
}

/** Shared look: long, thin tab hanging from the top edge, rounded at the bottom. */
@Composable
private fun NotchSurface(modifier: Modifier, content: @Composable () -> Unit) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(bottomStart = 16.dp, bottomEnd = 16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.93f),
        tonalElevation = 3.dp,
    ) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 3.dp, bottom = 5.dp)) { content() }
    }
}

@Composable
private fun NotchTitle(title: String, info: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            title,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.primary,
            maxLines = 1,
            modifier = Modifier.weight(1f, fill = false),
        )
        if (info.isNotBlank()) {
            Text("  ·  $info", style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
        }
    }
}

@Composable
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
private fun NotchTicker(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurface,
        maxLines = 1,
        modifier = Modifier.fillMaxWidth().basicMarquee(iterations = Int.MAX_VALUE),
    )
}


private val BAND_STRIP_HEIGHT = 15.dp

/** Band plan colors, readable over any waterfall palette (also used for the legend in Settings). */
val BandKind.color: Color
    get() = when (this) {
        BandKind.HAM -> Color(0xFF3DBE7A)
        BandKind.BROADCAST -> Color(0xFF4F8EF7)
        BandKind.FM -> Color(0xFF6E6BF2)
        BandKind.DAB -> Color(0xFFE8628F)
        BandKind.TV -> Color(0xFF8C93A3)
        BandKind.CB -> Color(0xFFF29A38)
        BandKind.PMR -> Color(0xFFE8C547)
        BandKind.AIR -> Color(0xFFB27CF2)
        BandKind.MARINE -> Color(0xFF26B9C9)
        BandKind.SAT -> Color(0xFFEF5B4C)
    }
