package app.littlesdr.ui.components

import app.littlesdr.data.FlightRoute
import app.littlesdr.data.Airport
import app.littlesdr.data.AircraftDetails
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.draw.rotate
import androidx.compose.material.icons.filled.Flight
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.DeleteSweep
import androidx.compose.material.icons.filled.Warning
import app.littlesdr.ui.theme.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.AdsbState
import app.littlesdr.engine.Aircraft
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt

private fun thousands(v: Int): String = String.format(Locale.FRANCE, "%,d", v).replace(' ', ' ').replace(' ', ' ')

private fun altitudeText(a: Aircraft): String = when {
    a.onGround -> "au sol"
    a.altitude != null -> "${thousands(a.altitude!!)} ft"
    else -> "—"
}

/** 8 compass points in French */
private fun compass(deg: Double): String = listOf("N", "NE", "E", "SE", "S", "SO", "O", "NO")[((deg + 22.5) / 45).toInt().mod(8)]

/** Distress first, then the closest (or, without position, the most recently heard) */
private fun ordered(list: List<Aircraft>): List<Aircraft> = list.sortedWith(
    compareBy<Aircraft>({ it.distress == null }, { it.nearDist ?: Double.MAX_VALUE }, { it.seen }),
)

/**
 * Right side in the ADS-B space: reception summary, the picked aircraft in detail, then
 * every aircraft heard (tap one to pick it on the map).
 */
@Composable
fun AdsbPanel(
    state: AdsbState,
    selected: String?,
    routes: Map<String, FlightRoute?>,
    airframe: AircraftDetails?,
    photo: ImageBitmap?,
    onSelect: (String?) -> Unit,
    onClear: () -> Unit,
) {
    val routeOf = { a: Aircraft -> routes[a.callsign.trim()] }
    val cs = MaterialTheme.colorScheme
    val onMap = state.aircraft.count { it.position != null }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            if (!state.active) "1090 MHz · ADS-B et Mode S"
            else "${state.aircraft.size} avion(s) · $onMap sur la carte · ${String.format(Locale.FRANCE, "%.1f", state.rate)} msg/s",
            style = MaterialTheme.typography.bodyMedium,
            color = cs.onSurfaceVariant,
        )
        if (state.active && state.homePoint == null) {
            Text(
                "Indique ton locator (Réglages → Opérateur) pour les distances et les cercles autour de toi.",
                style = MaterialTheme.typography.bodySmall,
                color = cs.onSurfaceVariant,
            )
        }
        state.aircraft.firstOrNull { it.icao == selected }?.let { a -> AircraftCard(a, routeOf(a), airframe, photo, onClose = { onSelect(null) }) }
        ordered(state.aircraft).forEach { a -> AircraftRow(a, routeOf(a), a.icao == selected) { onSelect(a.icao) } }
        if (state.aircraft.isNotEmpty()) {
            FilledTonalButton(onClick = onClear, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Default.DeleteSweep, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Vider la liste")
            }
        }
    }
}

@Composable
private fun AircraftRow(a: Aircraft, route: FlightRoute?, selected: Boolean, onClick: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    val dot = if (a.distress != null) cs.error else altitudeColor(a.altitude, a.onGround, cs.outline)
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(if (selected) cs.secondaryContainer else cs.surfaceContainer)
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(10.dp).clip(CircleShape).background(if (a.position != null) dot else dot.copy(alpha = 0.35f)))
        Spacer(Modifier.width(8.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(a.name, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold, maxLines = 1)
                if (a.flag.isNotEmpty()) Text("  ${a.flag}", style = MaterialTheme.typography.bodyMedium)
                if (a.distress != null) {
                    Spacer(Modifier.width(4.dp))
                    Icon(Icons.Default.Warning, a.distress, Modifier.size(16.dp), tint = cs.error)
                }
            }
            Text(
                listOfNotNull(
                    // Departure -> arrival; "?" when the aircraft is far off that way
                    route?.let { r -> "${r.origin.code} → ${r.destination.code}" + if (r.plausible(a.lat, a.lon) == false) " ?" else "" },
                    altitudeText(a),
                    a.speed?.let { "${it.roundToInt()} kt" },
                    a.vrate?.takeIf { abs(it) >= 300 }?.let { if (it > 0) "↗" else "↘" },
                ).joinToString(" · "),
                style = MaterialTheme.typography.bodySmall,
                color = cs.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(
            a.nearDist?.let { "${it.roundToInt()} km" } ?: "",
            style = MaterialTheme.typography.bodyMedium,
            color = cs.onSurfaceVariant,
        )
    }
}

@Composable
private fun AircraftCard(a: Aircraft, route: FlightRoute?, airframe: AircraftDetails?, photo: ImageBitmap?, onClose: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    Surface(color = cs.secondaryContainer, shape = RoundedCornerShape(16.dp), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 14.dp, end = 4.dp, top = 6.dp, bottom = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(a.name, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
                    Text(
                        listOf(
                            "ICAO ${a.icao}" + if (a.nonIcao) " (anonyme)" else "",
                            listOf(a.flag, a.countryName).filter { it.isNotEmpty() }.joinToString(" "),
                        ).filter { it.isNotBlank() }.joinToString(" · "),
                        style = MaterialTheme.typography.bodySmall,
                        color = cs.onSecondaryContainer,
                    )
                }
                IconButton(onClick = onClose) { Icon(Icons.Default.Close, "Fermer") }
            }
            a.distress?.let {
                Surface(color = cs.error, shape = RoundedCornerShape(8.dp), modifier = Modifier.padding(end = 10.dp, top = 4.dp, bottom = 4.dp)) {
                    Row(Modifier.padding(horizontal = 8.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.Warning, null, Modifier.size(16.dp), tint = cs.onError)
                        Spacer(Modifier.width(6.dp))
                        Text(it, color = cs.onError, style = MaterialTheme.typography.labelLarge)
                    }
                }
            }
            photo?.let {
                Image(
                    it, "Photo de l'avion", contentScale = ContentScale.Crop,
                    modifier = Modifier.padding(end = 10.dp, top = 6.dp, bottom = 4.dp).fillMaxWidth().aspectRatio(16f / 9f).clip(RoundedCornerShape(10.dp)),
                )
            }
            route?.let { r ->
                val doubtful = r.plausible(a.lat, a.lon) == false
                Column(Modifier.padding(end = 10.dp, top = 6.dp, bottom = 4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        AirportCell(r.origin, Modifier.weight(1f))
                        Icon(Icons.Default.Flight, null, Modifier.padding(horizontal = 6.dp).size(20.dp).rotate(90f), tint = cs.onSecondaryContainer)
                        AirportCell(r.destination, Modifier.weight(1f), end = true)
                    }
                    if (r.airline.isNotBlank()) Text(r.airline, style = MaterialTheme.typography.bodySmall, color = cs.onSecondaryContainer.copy(alpha = 0.8f))
                    if (doubtful) Text(
                        "Route de l'annuaire en ligne, mais l'avion n'est pas sur ce trajet : elle est sans doute périmée.",
                        style = MaterialTheme.typography.bodySmall,
                        color = cs.error,
                    )
                }
            }
            Column(Modifier.padding(end = 10.dp, top = 4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                airframe?.let { f ->
                    val model = listOf(f.manufacturer, f.type).filter { it.isNotBlank() }.joinToString(" ")
                    if (model.isNotBlank()) Detail("Appareil", model)
                    if (f.registration.isNotBlank()) Detail("Immatriculation", f.registration)
                    if (f.owner.isNotBlank() && route?.airline != f.owner) Detail("Exploitant", f.owner)
                }
                if (a.categoryLabel.isNotEmpty()) Detail("Type", a.categoryLabel)
                Detail(
                    "Altitude",
                    altitudeText(a) + (a.altitude?.takeIf { !a.onGround }?.let { " · ${thousands((it * 0.3048).roundToInt())} m" } ?: ""),
                )
                if (a.gnssAlt != null && a.alt != null) Detail("Altitude GPS", "${thousands(a.gnssAlt)} ft")
                a.vrate?.let { v ->
                    Detail("Vario", when {
                        abs(v) < 100 -> "palier"
                        v > 0 -> "monte ${thousands(v)} ft/min"
                        else -> "descend ${thousands(-v)} ft/min"
                    })
                }
                a.speed?.let { s ->
                    Detail(if (a.airspeed) "Vitesse air" else "Vitesse sol", "${s.roundToInt()} kt · ${(s * 1.852).roundToInt()} km/h")
                }
                a.direction?.let { d ->
                    Detail(if (a.track != null) "Direction" else "Cap", "${d.roundToInt()}° (${compass(d)})")
                }
                a.squawk?.let { Detail("Transpondeur", it) }
                val d = a.nearDist
                if (d != null && a.bearing != null) Detail("Distance", "${d.roundToInt()} km au ${compass(a.bearing)} (${a.bearing.roundToInt()}°)")
                a.position?.let { Detail("Position", String.format(Locale.ROOT, "%.4f, %.4f", it.lat, it.lon), mono = true) }
                Detail("Signal", String.format(Locale.FRANCE, "%.1f dBFS · %d msg", a.rssi, a.msgs))
                if (a.seen >= 5) Detail("Entendu", String.format(Locale.FRANCE, "il y a %.0f s", a.seen))
            }
        }
    }
}

@Composable
private fun Detail(name: String, value: String, mono: Boolean = false) {
    Row(Modifier.fillMaxWidth()) {
        Text(name, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = 0.75f), modifier = Modifier.width(124.dp))
        Text(value, style = MaterialTheme.typography.bodyMedium, fontFamily = if (mono) FontFamily.Monospace else null)
    }
}

/** "MXP" big, "Milan" under it */
@Composable
private fun AirportCell(ap: Airport, modifier: Modifier, end: Boolean = false) {
    Column(modifier, horizontalAlignment = if (end) Alignment.End else Alignment.Start) {
        Text(ap.code, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        Text(ap.place, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
