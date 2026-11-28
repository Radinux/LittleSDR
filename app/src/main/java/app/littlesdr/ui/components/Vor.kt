package app.littlesdr.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.littlesdr.engine.GeoPoint
import app.littlesdr.engine.VOR_BAND
import app.littlesdr.engine.VOR_STEP
import app.littlesdr.engine.VorState
import app.littlesdr.engine.VorStation
import app.littlesdr.engine.bearingDeg
import app.littlesdr.engine.compassPoint
import app.littlesdr.engine.distanceKm
import app.littlesdr.ui.theme.FilledTonalIconButton
import app.littlesdr.ui.theme.FilterChip
import app.littlesdr.ui.theme.pillBorder
import java.util.Locale
import kotlin.math.cos
import kotlin.math.sin

private fun deg(d: Double) = String.format(Locale.FRANCE, "%03.0f°", ((d % 360) + 360) % 360)

/**
 * Left side in the VOR space: a compass rose, magnetic north up, the beacon in the middle and
 * an arrow along the radial we are on, its value in big.
 */
@Composable
fun VorCompass(v: VorState, station: VorStation?, playing: Boolean, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    val measurer = rememberTextMeasurer()
    val radial = v.radial.takeIf { playing }
    Surface(color = cs.surfaceContainerLowest, shape = RoundedCornerShape(20.dp), border = pillBorder(), modifier = modifier) {
        Box(Modifier.fillMaxSize().padding(8.dp), contentAlignment = Alignment.Center) {
            Canvas(Modifier.fillMaxSize().padding(bottom = if (radial == null) 34.dp else 0.dp)) {
                val c = center
                val r = size.minDimension / 2 * 0.80f
                // The rose: a tick every 10°, longer every 30°, the cardinal points
                drawCircle(cs.outlineVariant, r, c, style = Stroke(1.5.dp.toPx()))
                for (a in 0 until 360 step 10) {
                    val t = Math.toRadians(a.toDouble())
                    val long = a % 30 == 0
                    val r0 = r - (if (long) 12.dp else 6.dp).toPx()
                    drawLine(
                        if (long) cs.onSurfaceVariant else cs.outlineVariant,
                        Offset(c.x + r0 * sin(t).toFloat(), c.y - r0 * cos(t).toFloat()),
                        Offset(c.x + r * sin(t).toFloat(), c.y - r * cos(t).toFloat()),
                        strokeWidth = (if (long) 2.dp else 1.dp).toPx(),
                    )
                }
                listOf(0 to "N", 90 to "E", 180 to "S", 270 to "O").forEach { (a, s) ->
                    val t = Math.toRadians(a.toDouble())
                    val lr = r + 12.dp.toPx()
                    val lay = measurer.measure(s, TextStyle(fontSize = 13.sp, fontWeight = FontWeight.Bold, color = if (a == 0) cs.primary else cs.onSurfaceVariant))
                    drawText(lay, topLeft = Offset(c.x + lr * sin(t).toFloat() - lay.size.width / 2, c.y - lr * cos(t).toFloat() - lay.size.height / 2))
                }
                // The beacon, and us out along the radial
                drawCircle(cs.primary, 5.dp.toPx(), c)
                if (radial != null) {
                    val t = Math.toRadians(radial)
                    val tip = Offset(c.x + (r - 16.dp.toPx()) * sin(t).toFloat(), c.y - (r - 16.dp.toPx()) * cos(t).toFloat())
                    drawLine(cs.primary, c, tip, strokeWidth = 4.dp.toPx(), cap = StrokeCap.Round)
                    val head = 14.dp.toPx()
                    val p = Path().apply {
                        moveTo(c.x + r * sin(t).toFloat(), c.y - r * cos(t).toFloat())
                        lineTo(tip.x + head / 2 * cos(t).toFloat(), tip.y + head / 2 * sin(t).toFloat())
                        lineTo(tip.x - head / 2 * cos(t).toFloat(), tip.y - head / 2 * sin(t).toFloat())
                        close()
                    }
                    drawPath(p, cs.primary)
                }
            }
            if (radial != null) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    // Over the rose, out of the arrow's way: a dark disc behind the figures
                    Surface(color = cs.surfaceContainerLowest.copy(alpha = 0.85f), shape = RoundedCornerShape(16.dp)) {
                        Column(Modifier.padding(horizontal = 14.dp, vertical = 4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(deg(radial), style = MaterialTheme.typography.displayMedium, fontWeight = FontWeight.Bold)
                            Text("radial ${v.ident.ifBlank { station?.ident ?: "" }}".trim(), style = MaterialTheme.typography.titleMedium, color = cs.onSurfaceVariant)
                        }
                    }
                }
            } else {
                // The hint under the rose, not across it
                Column(Modifier.align(Alignment.BottomCenter), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        if (!playing) "Appuie sur ▶" else "Recherche du VOR…",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                        textAlign = TextAlign.Center,
                    )
                    Text(
                        "Antenne VHF dégagée ; un VOR porte loin en altitude, peu au sol.",
                        style = MaterialTheme.typography.bodySmall,
                        color = cs.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

/** Right side in the VOR space: tuning, the beacon, what is received, the beacons around */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun VorPanel(
    frequency: Double,
    v: VorState,
    playing: Boolean,
    station: VorStation?,
    stations: List<VorStation>,
    me: GeoPoint?,
    onTune: (Double) -> Unit,
) {
    val cs = MaterialTheme.colorScheme
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        // 50 kHz steps through the band
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            FilledTonalIconButton(onClick = { onTune((frequency - VOR_STEP).coerceIn(VOR_BAND)) }, modifier = Modifier.size(44.dp)) { Icon(Icons.Default.Remove, "50 kHz plus bas") }
            Text(
                String.format(Locale.FRANCE, "%.2f MHz", frequency / 1e6),
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
                modifier = Modifier.weight(1f),
            )
            FilledTonalIconButton(onClick = { onTune((frequency + VOR_STEP).coerceIn(VOR_BAND)) }, modifier = Modifier.size(44.dp)) { Icon(Icons.Default.Add, "50 kHz plus haut") }
        }
        if (station != null) {
            Column {
                Text("${station.ident} · ${station.name}", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                val where = me?.let { "à ${distanceKm(it, station.position).toInt()} km au ${compassPoint(bearingDeg(it, station.position))} · " } ?: ""
                Text(where + station.mhz, style = MaterialTheme.typography.bodyMedium, color = cs.onSurfaceVariant)
            }
        }
        Text("Réception", style = MaterialTheme.typography.titleSmall)
        val on = playing && v.active
        Stat("Indicatif (morse)", v.ident.ifBlank { v.morse.takeIf { on && it.isNotBlank() }?.let { "« $it »" } ?: "—" })
        Stat("Radial (magnétique)", v.radial?.takeIf { on }?.let(::deg) ?: "—")
        Stat("Cap vers la balise", v.toBeacon?.takeIf { on }?.let(::deg) ?: "—")
        Stat(
            "Signal 30 Hz",
            if (!on || v.depth <= 0) "—" else String.format(Locale.FRANCE, "%.0f %% · réf. ±%.0f Hz · %s", v.depth * 100, v.deviation, if (v.steadiness > 0.97) "stable" else "instable"),
        )
        Text("Balises proches", style = MaterialTheme.typography.titleSmall)
        if (me == null) {
            Text("Indique ton locator (Réglages → Opérateur) pour voir les balises autour de toi.", style = MaterialTheme.typography.bodyMedium, color = cs.onSurfaceVariant)
        } else {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                app.littlesdr.engine.VorStations.nearest(stations, me).forEach { s ->
                    FilterChip(
                        selected = kotlin.math.abs(frequency - s.hz) < 1_000,
                        onClick = { onTune(s.hz) },
                        label = {
                            Text("${s.ident} · ${String.format(Locale.FRANCE, "%.2f", s.hz / 1e6)} · ${distanceKm(me, s.position).toInt()} km")
                        },
                    )
                }
            }
        }
        Spacer(Modifier.width(1.dp))
    }
}
