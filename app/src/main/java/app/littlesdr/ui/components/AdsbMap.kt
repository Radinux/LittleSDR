package app.littlesdr.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Flight
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.littlesdr.engine.AdsbState
import app.littlesdr.engine.Aircraft
import app.littlesdr.engine.GeoPoint
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min

/** Altitude colour (tar1090-like): orange near the ground, through green and blue, to violet at 40 000 ft */
fun altitudeColor(alt: Int?, ground: Boolean, unknown: Color): Color = when {
    ground -> Color(0xFF9E9E9E)
    alt == null -> unknown
    else -> Color.hsv(20f + 280f * (alt / 40_000f).coerceIn(0f, 1f), 0.78f, 0.92f)
}

/** Plane silhouette pointing up, in a unit square centred on 0 */
private val PLANE: List<Offset> = listOf(
    Offset(0f, -1f), Offset(0.11f, -0.82f), Offset(0.12f, -0.3f), Offset(0.95f, 0.12f), Offset(0.95f, 0.3f),
    Offset(0.12f, 0.1f), Offset(0.09f, 0.62f), Offset(0.38f, 0.82f), Offset(0.38f, 0.96f), Offset(0f, 0.86f),
)

private fun planePath(c: Offset, r: Float) = Path().apply {
    val right = PLANE
    val left = PLANE.reversed().drop(1).dropLast(1).map { Offset(-it.x, it.y) }
    (right + left).forEachIndexed { i, p -> if (i == 0) moveTo(c.x + p.x * r, c.y + p.y * r) else lineTo(c.x + p.x * r, c.y + p.y * r) }
    close()
}

/**
 * Aircraft around the receiver: country outlines, range rings, tracks coloured by altitude,
 * planes along their track with their flight and flight level. Fits everything until the
 * user pans or zooms; double tap fits again. Tap a plane to pick it.
 */
@Composable
fun AdsbMap(
    state: AdsbState,
    selected: String?,
    playing: Boolean,
    onSelect: (String?) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val land = remember { WorldMap.path(context) }
    val measurer = rememberTextMeasurer(cacheSize = 128)
    val cs = MaterialTheme.colorScheme

    // View: centre and pixels per degree of latitude
    var center by remember { mutableStateOf(GeoPoint(46.5, 2.5)) }
    var k by remember { mutableFloatStateOf(0f) }
    var userMoved by remember { mutableStateOf(false) }
    val current by rememberUpdatedState(state)
    val shown = remember { ArrayList<Pair<Offset, String>>() } // planes drawn, for taps

    val ocean = lerp(cs.surfaceContainerLow, cs.primaryContainer, 0.18f)
    val landFill = lerp(cs.surfaceContainerHighest, cs.secondaryContainer, 0.35f)
    val landEdge = lerp(cs.outline, cs.onSurface, 0.15f)
    val labelStyle = TextStyle(fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = cs.onSurface)
    val subStyle = TextStyle(fontSize = 10.sp, color = cs.onSurfaceVariant)
    val ringStyle = TextStyle(fontSize = 10.sp, color = cs.onSurfaceVariant)

    val positioned = state.aircraft.filter { it.position != null }
    Box(modifier.clip(RoundedCornerShape(20.dp))) {
        Canvas(
            Modifier
                .fillMaxSize()
                .pointerInput(Unit) {
                    detectTransformGestures { centroid, pan, zoom, _ ->
                        if (k <= 0f) return@detectTransformGestures
                        val cosLat = cos(center.lat * PI / 180).toFloat().coerceAtLeast(0.1f)
                        // Geo point under the fingers stays there while zooming, then pan
                        val gx = center.lon + (centroid.x - size.width / 2f) / (k * cosLat)
                        val gy = center.lat - (centroid.y - size.height / 2f) / k
                        val nk = (k * zoom).coerceIn(4f, 4000f)
                        val lon = gx - (centroid.x + pan.x - size.width / 2f) / (nk * cosLat)
                        val lat = (gy + (centroid.y + pan.y - size.height / 2f) / nk).coerceIn(-85.0, 85.0)
                        center = GeoPoint(lat, lon)
                        k = nk
                        userMoved = true
                    }
                }
                .pointerInput(Unit) {
                    detectTapGestures(
                        onTap = { t ->
                            val hit = shown.minByOrNull { hypot(it.first.x - t.x, it.first.y - t.y) }
                            onSelect(hit?.takeIf { hypot(it.first.x - t.x, it.first.y - t.y) < 28.dp.toPx() }?.second)
                        },
                        onDoubleTap = { userMoved = false; k = 0f },
                    )
                },
        ) {
            if (k <= 0f || !userMoved) fit(current)?.let { (c, s) -> center = c; k = s }
            val cosLat = cos(center.lat * PI / 180).toFloat().coerceAtLeast(0.1f)
            val kx = k * cosLat
            fun screen(lat: Double, lon: Double) =
                Offset(size.width / 2f + ((lon - center.lon) * kx).toFloat(), size.height / 2f - ((lat - center.lat) * k).toFloat())

            drawRect(ocean)
            // Land, outlines in degrees (x = lon + 180, y = 90 - lat)
            val tx = size.width / 2f - ((180.0 + center.lon) * kx).toFloat()
            val ty = size.height / 2f - ((90.0 - center.lat) * k).toFloat()
            withTransform({ translate(tx, ty); scale(kx, k, Offset.Zero) }) {
                drawPath(land, landFill)
                drawPath(land, landEdge, style = Stroke(width = 1.4.dp.toPx() / k, join = StrokeJoin.Round))
            }

            // Range rings around the receiver, step chosen for the zoom
            val home = current.homePoint
            val pxPerKm = k / 111.2f
            if (home != null) {
                val h = screen(home.lat, home.lon)
                val step = listOf(10, 25, 50, 100, 200).firstOrNull { it * pxPerKm > 70.dp.toPx() } ?: 400
                val dash = PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(), 5.dp.toPx()))
                for (i in 1..8) {
                    val r = i * step * pxPerKm
                    if (r > hypot(size.width, size.height) * 1.2f) break
                    drawCircle(cs.onSurface.copy(alpha = 0.22f), r, h, style = Stroke(1.dp.toPx(), pathEffect = dash))
                    val t = measurer.measure("${i * step} km", ringStyle)
                    drawText(t, topLeft = Offset(h.x + r * 0.707f + 2.dp.toPx(), h.y - r * 0.707f - t.size.height))
                }
            } else {
                scaleBar(pxPerKm, measurer.measure("", ringStyle).size.height.toFloat()) { km ->
                    measurer.measure("$km km", ringStyle)
                }
            }

            // Tracks: one segment per point pair, coloured by altitude
            for (a in positioned) {
                val pts = a.trail.mapNotNull { p -> if (p.size >= 2 && p[0] != null && p[1] != null) Triple(p[0]!!, p[1]!!, p.getOrNull(2)) else null }
                val w = if (a.icao == selected) 3.dp.toPx() else 1.6.dp.toPx()
                val alpha = if (a.icao == selected) 1f else 0.7f
                for (i in 1 until pts.size) {
                    val (la0, lo0, _) = pts[i - 1]
                    val (la1, lo1, al) = pts[i]
                    val c = altitudeColor(al?.toInt(), a.onGround, cs.onSurfaceVariant).copy(alpha = alpha)
                    drawLine(c, screen(la0, lo0), screen(la1, lo1), w, cap = StrokeCap.Round)
                }
            }

            // Receiver
            home?.let { hp ->
                val p = screen(hp.lat, hp.lon)
                val r = 7.dp.toPx()
                val house = Path().apply {
                    moveTo(p.x, p.y - r); lineTo(p.x + r, p.y); lineTo(p.x + r * 0.7f, p.y)
                    lineTo(p.x + r * 0.7f, p.y + r * 0.8f); lineTo(p.x - r * 0.7f, p.y + r * 0.8f)
                    lineTo(p.x - r * 0.7f, p.y); lineTo(p.x - r, p.y); close()
                }
                drawPath(house, cs.tertiary)
                drawPath(house, cs.onTertiary, style = Stroke(1.5.dp.toPx(), join = StrokeJoin.Round))
            }

            // Planes, the picked one and distress on top; labels where they fit
            shown.clear()
            val placed = ArrayList<Rect>()
            val order = positioned.sortedWith(compareBy<Aircraft>({ it.icao == selected }, { it.distress != null }))
            for (a in order) {
                val pos = a.position ?: continue
                val p = screen(pos.lat, pos.lon)
                if (p.x < -40 || p.y < -40 || p.x > size.width + 40 || p.y > size.height + 40) continue
                shown += p to a.icao
                val isSel = a.icao == selected
                val stale = (a.seenPos ?: 0.0) > 20.0
                val r = (if (isSel) 13 else 10).dp.toPx()
                val fill = if (a.distress != null) cs.error else altitudeColor(a.altitude, a.onGround, cs.onSurfaceVariant)
                val alpha = if (stale) 0.45f else 1f
                if (isSel) drawCircle(cs.primary.copy(alpha = 0.25f), r * 1.9f, p)
                if (a.distress != null) drawCircle(cs.error, r * 1.7f, p, style = Stroke(2.dp.toPx()))
                rotate((a.direction ?: 0.0).toFloat(), pivot = p) {
                    val plane = planePath(p, r)
                    drawPath(planePath(p + Offset(1.dp.toPx(), 1.5.dp.toPx()), r), Color.Black.copy(alpha = 0.3f * alpha))
                    drawPath(plane, fill.copy(alpha = alpha))
                    drawPath(plane, cs.surface.copy(alpha = alpha), style = Stroke(1.2.dp.toPx(), join = StrokeJoin.Round))
                }
                placed += Rect(p, r)
                // Flight, then flight level (or "sol")
                val l1 = measurer.measure(a.name, labelStyle)
                val alt = when {
                    a.onGround -> "sol"
                    a.altitude != null -> "FL" + String.format(java.util.Locale.ROOT, "%03d", (a.altitude!! + 50) / 100)
                    else -> ""
                }
                val l2 = measurer.measure(alt, subStyle)
                val tl = Offset(p.x + r + 4.dp.toPx(), p.y - (l1.size.height + l2.size.height) / 2f)
                val box = Rect(
                    tl - Offset(3.dp.toPx(), 1.dp.toPx()),
                    Size(max(l1.size.width, l2.size.width) + 6.dp.toPx(), (l1.size.height + l2.size.height) + 2.dp.toPx()),
                )
                if (isSel || placed.none { it.overlaps(box) }) {
                    placed += box
                    drawRoundRect(cs.surface.copy(alpha = 0.78f * alpha), box.topLeft, box.size, CornerRadius(6.dp.toPx()))
                    drawText(l1, topLeft = tl)
                    drawText(l2, topLeft = tl + Offset(0f, l1.size.height.toFloat()))
                }
            }
        }

        // Nothing to show yet: what to do about it
        if (positioned.isEmpty()) {
            Surface(
                color = cs.surface.copy(alpha = 0.88f),
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.align(Alignment.Center).padding(24.dp),
            ) {
                Column(Modifier.padding(horizontal = 20.dp, vertical = 14.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(Icons.Default.Flight, null, Modifier.size(36.dp), tint = cs.onSurfaceVariant)
                    Text(
                        when {
                            !playing -> "Appuie sur ▶ pour écouter le 1090 MHz"
                            state.aircraft.isNotEmpty() -> "${state.aircraft.size} avion(s) entendu(s), position en attente…"
                            else -> "À l'écoute des avions…"
                        },
                        style = MaterialTheme.typography.titleMedium,
                        textAlign = TextAlign.Center,
                    )
                    Text(
                        "Antenne verticale de 6,9 cm (quart d'onde), dehors ou à une fenêtre bien dégagée. Gain élevé.",
                        style = MaterialTheme.typography.bodySmall,
                        color = cs.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

/** Centre and pixels per degree framing the aircraft and the receiver (100 km around at least) */
private fun DrawScope.fit(s: AdsbState): Pair<GeoPoint, Float>? {
    if (size.width <= 0f || size.height <= 0f) return null
    val planes = s.aircraft.mapNotNull { it.position }
    // The receiver only when it is near the aircraft (a network receiver may be far away)
    val home = s.homePoint?.takeIf { h ->
        planes.isEmpty() || hypot(planes.map { it.lat }.average() - h.lat, (planes.map { it.lon }.average() - h.lon) * cos(h.lat * PI / 180)) < 5.0
    }
    val pts = planes + listOfNotNull(home)
    if (pts.isEmpty()) return GeoPoint(46.5, 2.5) to min(size.height / 12f, size.width / 14f)
    val minLat = pts.minOf { it.lat }; val maxLat = pts.maxOf { it.lat }
    val minLon = pts.minOf { it.lon }; val maxLon = pts.maxOf { it.lon }
    val c = GeoPoint((minLat + maxLat) / 2, (minLon + maxLon) / 2)
    val cosLat = cos(c.lat * PI / 180).coerceAtLeast(0.1)
    val dLat = max(maxLat - minLat, 1.8) * 1.25
    val dLon = max((maxLon - minLon) * cosLat, 1.8) * 1.25
    return c to min(size.height / dLat, size.width / dLon).toFloat()
}

/** Scale bar at the bottom left when there is no receiver position for rings */
private fun DrawScope.scaleBar(pxPerKm: Float, textH: Float, label: (Int) -> androidx.compose.ui.text.TextLayoutResult) {
    val km = listOf(5, 10, 25, 50, 100, 200, 500).firstOrNull { it * pxPerKm > 60.dp.toPx() } ?: 1000
    val len = km * pxPerKm
    val y = size.height - 14.dp.toPx()
    val x = 14.dp.toPx()
    drawLine(Color.Black.copy(alpha = 0.6f), Offset(x, y), Offset(x + len, y), 2.dp.toPx(), cap = StrokeCap.Round)
    val t = label(km)
    drawText(t, topLeft = Offset(x, y - textH - 4.dp.toPx()))
}
