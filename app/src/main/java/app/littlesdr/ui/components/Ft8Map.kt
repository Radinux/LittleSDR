package app.littlesdr.ui.components

import android.content.Context
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.littlesdr.engine.Ft8Station
import app.littlesdr.engine.Ft8Traffic
import app.littlesdr.engine.GeoPoint
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.ln
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/** Country outlines from assets/world.bin (tools/make_world_map.py), as one Path in degrees. */
private object WorldMap {
    private var path: Path? = null

    /** x = lon + 180 (0..360), y = 90 - lat (0..180): equirectangular, north up. */
    fun path(context: Context): Path = path ?: Path().also { p ->
        val bytes = context.assets.open("world.bin").use { it.readBytes() }
        val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        repeat(b.int) {
            val n = b.int
            for (i in 0 until n) {
                val x = b.float + 180f
                val y = 90f - b.float
                if (i == 0) p.moveTo(x, y) else p.lineTo(x, y)
            }
            p.close()
        }
        path = p
    }
}

private fun GeoPoint.x() = (lon + 180.0).toFloat()
private fun GeoPoint.y() = (90.0 - lat).toFloat()

/**
 * Simple "cartoon" map of FT8 traffic: rounded thick-outlined land, stations as dots (CQ pulses),
 * curved arrows from sender to recipient fading over two minutes, home at the operator locator.
 * Pan / pinch to move, double tap to fit everything again.
 */
@Composable
fun Ft8Map(traffic: Ft8Traffic, home: GeoPoint?, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val land = remember { WorldMap.path(context) }
    val measurer = rememberTextMeasurer(cacheSize = 64)
    val cs = MaterialTheme.colorScheme

    // View: pixels per degree + offset of the world origin in pixels
    var scale by remember { mutableFloatStateOf(0f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    var userMoved by remember { mutableStateOf(false) }
    val clusters = remember { ArrayList<MapCluster>() } // last drawn groups, for taps

    val pulse by rememberInfiniteTransition(label = "cq").animateFloat(
        initialValue = 0f, targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(1600, easing = LinearEasing), RepeatMode.Restart),
        label = "pulse",
    )

    val ocean = lerp(cs.surfaceContainerLow, cs.primaryContainer, 0.18f)
    val landFill = lerp(cs.surfaceContainerHighest, cs.secondaryContainer, 0.35f)
    val landEdge = lerp(cs.outline, cs.onSurface, 0.15f)
    val shadow = Color.Black.copy(alpha = 0.25f)
    val labelStyle = TextStyle(fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = cs.onSurface)
    val countStyle = TextStyle(fontSize = 12.sp, fontWeight = FontWeight.Bold)

    Canvas(
        modifier
            .pointerInput(Unit) {
                detectTransformGestures { centroid, pan, zoom, _ ->
                    if (scale <= 0f) return@detectTransformGestures
                    val newScale = (scale * zoom).coerceIn(1.5f, 400f)
                    // Zoom around the fingers, then pan
                    offset = centroid - (centroid - offset) * (newScale / scale) + pan
                    scale = newScale
                    userMoved = true
                }
            }
            .pointerInput(Unit) {
                detectTapGestures(
                    // Tap a bubble: zoom ×3 on it so its stations spread out
                    onTap = { t ->
                        val hit = clusters.firstOrNull { it.members.size > 1 && hypot(it.center.x - t.x, it.center.y - t.y) < 30.dp.toPx() }
                        if (hit != null && scale > 0f) {
                            val ns = (scale * 3f).coerceAtMost(400f)
                            offset = hit.center - (hit.center - offset) * (ns / scale) + (Offset(size.width / 2f, size.height / 2f) - hit.center)
                            scale = ns
                            userMoved = true
                        }
                    },
                    onDoubleTap = { userMoved = false; scale = 0f },
                )
            },
    ) {
        // Fit on first draw (and after double tap) unless the user moved the map
        if (scale <= 0f || !userMoved) fit(traffic, home)?.let { (s, o) -> scale = s; offset = o }
        val s = scale
        fun screen(p: GeoPoint) = Offset(offset.x + p.x() * s, offset.y + p.y() * s)

        drawRect(ocean)

        // Graticule every 30°
        val grid = cs.onSurface.copy(alpha = 0.06f)
        for (lon in 0..360 step 30) {
            val x = offset.x + lon * s
            drawLine(grid, Offset(x, offset.y), Offset(x, offset.y + 180 * s), 1f)
        }
        for (lat in 0..180 step 30) {
            val y = offset.y + lat * s
            drawLine(grid, Offset(offset.x, y), Offset(offset.x + 360 * s, y), 1f)
        }

        // Line weights follow the zoom: thin coastlines at world scale (a degree is only a few
        // pixels there), the chunky cartoon outline when zoomed in. 0 = world view, 1 = close.
        val detail = ((s - 3f) / 17f).coerceIn(0f, 1f)
        fun weight(minDp: Float, maxDp: Float) = (minDp + (maxDp - minDp) * detail).dp.toPx()

        // Land: soft drop shadow, flat fill, rounded outline (cartoon look)
        val edge = weight(0.5f, 2.2f) / s
        val shadowDx = weight(0.6f, 2f)
        val shadowDy = weight(0.9f, 3f)
        withTransform({ translate(offset.x + shadowDx, offset.y + shadowDy); scale(s, s, Offset.Zero) }) {
            drawPath(land, shadow)
        }
        withTransform({ translate(offset.x, offset.y); scale(s, s, Offset.Zero) }) {
            drawPath(land, landFill)
            drawPath(land, landEdge, style = Stroke(width = edge, join = StrokeJoin.Round, cap = StrokeCap.Round))
        }

        // ---- Clustering in screen space: stations closer than ~36 dp merge into a counted bubble.
        // Zooming in spreads them apart, so clusters split on their own.
        val radius = 36.dp.toPx()
        val ordered = traffic.stations.sortedWith(compareByDescending<Ft8Station> { it.heard }.thenByDescending { it.lastHeard })
        val groups = ArrayList<MapCluster>()
        val clusterOf = HashMap<String, Int>()
        for (st in ordered) {
            val p = screen(st.pos)
            val idx = groups.indexOfFirst { hypot(it.center.x - p.x, it.center.y - p.y) < radius }
            if (idx >= 0) {
                groups[idx].members += st
                clusterOf[st.call] = idx
            } else {
                groups += MapCluster(p, mutableListOf(st))
                clusterOf[st.call] = groups.size - 1
            }
        }
        clusters.clear()
        clusters.addAll(groups)

        // ---- Links between groups: one arrow per pair, thicker with more exchanges, fading with age
        val newest = traffic.links.maxOfOrNull { it.time } ?: 0L
        data class Agg(var count: Int, var time: Long)
        val agg = LinkedHashMap<Pair<Int, Int>, Agg>()
        for (l in traffic.links) {
            val a = clusterOf[l.from] ?: continue
            val b = clusterOf[l.to] ?: continue
            if (a == b) continue
            val e = agg.getOrPut(a to b) { Agg(0, l.time) }
            e.count++
            e.time = maxOf(e.time, l.time)
        }
        for ((k, v) in agg) {
            val pa = groups[k.first].center
            val pb = groups[k.second].center
            if (hypot(pb.x - pa.x, pb.y - pa.y) < 4f) continue
            val age = ((newest - v.time) / 120_000f).coerceIn(0f, 1f)
            val color = cs.primary.copy(alpha = 0.85f - 0.65f * age)
            val w = weight(1.2f, 2.2f) * (1f + 0.35f * ln(v.count.toFloat()))
            arrow(pa, pb, color, w)
        }

        // Home
        home?.let { h ->
            val p = screen(h)
            val r = 7.dp.toPx()
            val house = Path().apply {
                moveTo(p.x, p.y - r); lineTo(p.x + r, p.y); lineTo(p.x + r * 0.7f, p.y)
                lineTo(p.x + r * 0.7f, p.y + r * 0.8f); lineTo(p.x - r * 0.7f, p.y + r * 0.8f)
                lineTo(p.x - r * 0.7f, p.y); lineTo(p.x - r, p.y); close()
            }
            drawPath(house, cs.tertiary)
            drawPath(house, cs.onTertiary, style = Stroke(1.5.dp.toPx(), join = StrokeJoin.Round))
        }

        // ---- Stations: bubbles for groups, dots + labels (without overlaps) for singles
        val placed = ArrayList<androidx.compose.ui.geometry.Rect>()
        for (g in groups) {
            if (g.members.size > 1) {
                val r = (11f + 3f * ln(g.members.size.toFloat())).dp.toPx()
                placed += androidx.compose.ui.geometry.Rect(g.center, r)
            }
        }
        for (g in groups) {
            val p = g.center
            if (p.x < -60 || p.y < -60 || p.x > size.width + 60 || p.y > size.height + 60) continue
            val anyCq = g.members.any { it.callingCq }
            if (g.members.size > 1) {
                val r = (11f + 3f * ln(g.members.size.toFloat())).dp.toPx()
                if (anyCq) drawCircle(cs.primary.copy(alpha = 0.45f * (1f - pulse)), r + 12.dp.toPx() * pulse, p, style = Stroke(2.dp.toPx()))
                drawCircle(Color.Black.copy(alpha = 0.3f), r, p + Offset(1.dp.toPx(), 2.dp.toPx()))
                drawCircle(if (anyCq) cs.primary else cs.secondaryContainer, r, p)
                drawCircle(cs.surface, r, p, style = Stroke(2.dp.toPx()))
                val t = measurer.measure("${g.members.size}", countStyle.copy(color = if (anyCq) cs.onPrimary else cs.onSecondaryContainer))
                drawText(t, topLeft = Offset(p.x - t.size.width / 2f, p.y - t.size.height / 2f))
                continue
            }
            val st = g.members.first()
            val r = 5.dp.toPx()
            if (st.callingCq) {
                drawCircle(cs.primary.copy(alpha = 0.5f * (1f - pulse)), r + 14.dp.toPx() * pulse, p, style = Stroke(2.dp.toPx()))
            }
            if (st.heard) {
                drawCircle(Color.Black.copy(alpha = 0.3f), r, p + Offset(1.dp.toPx(), 1.5.dp.toPx()))
                drawCircle(if (st.callingCq) cs.primary else cs.secondary, r, p)
                drawCircle(cs.surface, r, p, style = Stroke(1.5.dp.toPx()))
            } else {
                drawCircle(cs.onSurfaceVariant, r * 0.8f, p, style = Stroke(1.8.dp.toPx())) // only seen as a recipient
            }
            // Label only if it doesn't collide with another label or bubble
            val layout = measurer.measure(st.call, labelStyle)
            val tl = Offset(p.x + r + 3.dp.toPx(), p.y - layout.size.height / 2f)
            val box = androidx.compose.ui.geometry.Rect(
                tl - Offset(3.dp.toPx(), 1.dp.toPx()),
                Size(layout.size.width + 6.dp.toPx(), layout.size.height + 2.dp.toPx()),
            )
            if (placed.none { it.overlaps(box) }) {
                placed += box
                drawRoundRect(cs.surface.copy(alpha = 0.75f), box.topLeft, box.size, androidx.compose.ui.geometry.CornerRadius(6.dp.toPx()))
                drawText(layout, topLeft = tl)
            }
        }
    }
}

/** Scale/offset that frame all stations and home (Europe-sized minimum), or the world if empty. */
private fun DrawScope.fit(t: Ft8Traffic, home: GeoPoint?): Pair<Float, Offset>? {
    if (size.width <= 0f || size.height <= 0f) return null
    val pts = t.stations.map { it.pos } + listOfNotNull(home)
    var minX = 0f; var maxX = 360f; var minY = 10f; var maxY = 160f
    if (pts.isNotEmpty()) {
        minX = pts.minOf { it.x() }; maxX = pts.maxOf { it.x() }
        minY = pts.minOf { it.y() }; maxY = pts.maxOf { it.y() }
        // At least ~30° x 18° so a single station isn't zoomed to street level
        val cx = (minX + maxX) / 2; val cy = (minY + maxY) / 2
        val w = max(maxX - minX, 30f) * 1.25f; val h = max(maxY - minY, 18f) * 1.25f
        minX = cx - w / 2; maxX = cx + w / 2; minY = cy - h / 2; maxY = cy + h / 2
    }
    val s = min(size.width / (maxX - minX), size.height / (maxY - minY))
    val o = Offset(size.width / 2 - (minX + maxX) / 2 * s, size.height / 2 - (minY + maxY) / 2 * s)
    return s to o
}

/** Quadratic curve from a to b bulging sideways, with an arrow head at b. */
private fun DrawScope.arrow(a: Offset, b: Offset, color: Color, width: Float) {
    val mid = (a + b) / 2f
    val dx = b.x - a.x
    val dy = b.y - a.y
    val ctrl = Offset(mid.x - dy * 0.2f, mid.y + dx * 0.2f)
    val path = Path().apply { moveTo(a.x, a.y); quadraticTo(ctrl.x, ctrl.y, b.x, b.y) }
    drawPath(path, color, style = Stroke(width, cap = StrokeCap.Round))
    // Head oriented along the curve's end tangent (ctrl -> b)
    val ang = atan2(b.y - ctrl.y, b.x - ctrl.x)
    val len = width * 4.5f
    val head = Path().apply {
        moveTo(b.x, b.y)
        lineTo(b.x - len * cos(ang - 0.45f), b.y - len * sin(ang - 0.45f))
        lineTo(b.x - len * cos(ang + 0.45f), b.y - len * sin(ang + 0.45f))
        close()
    }
    drawPath(head, color)
}


/** Stations drawn as one bubble at [center] (screen pixels). */
private class MapCluster(val center: Offset, val members: MutableList<Ft8Station>)
