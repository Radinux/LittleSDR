package app.littlesdr.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.SatelliteAlt
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.DAB_CHANNELS
import app.littlesdr.engine.DRM_PRESETS
import app.littlesdr.engine.DabService
import app.littlesdr.engine.DigitalState
import app.littlesdr.engine.LRPT_SATELLITES
import app.littlesdr.engine.LrptState
import app.littlesdr.engine.Mode
import app.littlesdr.engine.dabChannelAt
import java.util.Locale

/** The three faces of the app, each with its own screen and receiver */
enum class AppSpace(val label: String, val icon: ImageVector) {
    SDR("SDR", Icons.Default.GraphicEq),
    DIGITAL("DAB+ / DRM", Icons.Default.Radio),
    LRPT("LRPT", Icons.Default.SatelliteAlt);

    fun accepts(m: Mode): Boolean = when (this) {
        SDR -> !m.isDigital
        DIGITAL -> m == Mode.DAB || m == Mode.DRM
        LRPT -> m == Mode.LRPT
    }
}

/**
 * Round button that blooms into the three spaces, the nearest first. Tapping one switches
 * the interface (and the receiver) to it.
 */
@Composable
fun SpaceFab(space: AppSpace, open: Boolean, onOpen: (Boolean) -> Unit, onSpace: (AppSpace) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier, horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        val items = AppSpace.entries
        items.forEachIndexed { i, s ->
            // Bottom item (nearest the button) opens first, the top one last
            val order = items.size - 1 - i
            AnimatedVisibility(
                visible = open,
                enter = fadeIn(tween(150, delayMillis = order * 45)) +
                    scaleIn(tween(240, delayMillis = order * 45, easing = FastOutSlowInEasing), initialScale = 0.5f, transformOrigin = TransformOrigin(1f, 1f)) +
                    slideInVertically(tween(240, delayMillis = order * 45, easing = FastOutSlowInEasing)) { it },
                exit = fadeOut(tween(110, delayMillis = i * 30)) +
                    scaleOut(tween(150, delayMillis = i * 30), targetScale = 0.5f, transformOrigin = TransformOrigin(1f, 1f)) +
                    slideOutVertically(tween(150, delayMillis = i * 30)) { it / 2 },
            ) {
                val current = s == space
                ExtendedFloatingActionButton(
                    onClick = { onOpen(false); onSpace(s) },
                    icon = { Icon(s.icon, null) },
                    text = { Text(s.label, fontWeight = if (current) FontWeight.Bold else FontWeight.Medium) },
                    containerColor = if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.secondaryContainer,
                    contentColor = if (current) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSecondaryContainer,
                    modifier = Modifier.height(48.dp),
                )
            }
        }
        val turn by animateFloatAsState(if (open) 90f else 0f, tween(240, easing = FastOutSlowInEasing), label = "fab")
        FloatingActionButton(
            onClick = { onOpen(!open) },
            containerColor = MaterialTheme.colorScheme.primaryContainer,
            modifier = Modifier.semantics { contentDescription = if (open) "Fermer" else "Changer d'interface (${space.label})" },
        ) {
            Crossfade(open, label = "fabIcon", modifier = Modifier.graphicsLayer { rotationZ = turn }) { o ->
                Icon(if (o) Icons.Default.Close else space.icon, null)
            }
        }
    }
}

private fun mhz(hz: Double) = String.format(Locale.FRANCE, "%.3f MHz", hz / 1e6)
private fun khz(hz: Double) = String.format(Locale.FRANCE, "%,.0f kHz", hz / 1e3)

// ---------------------------------------------------------------------------------- DAB+ / DRM

/** Left side in the digital space: what is playing, big */
@Composable
fun DigitalNowPlaying(d: DigitalState, slide: ImageBitmap?, mode: Mode?, frequency: Double, playing: Boolean, modifier: Modifier = Modifier) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(20.dp), modifier = modifier) {
        Row(Modifier.fillMaxSize().padding(20.dp), horizontalArrangement = Arrangement.spacedBy(20.dp)) {
            if (slide != null) {
                Image(
                    slide, "Diaporama de la station",
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxHeight().aspectRatio(4f / 3f, matchHeightConstraintsFirst = true).clip(RoundedCornerShape(12.dp)),
                )
            }
            Column(Modifier.weight(1f).fillMaxHeight(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                val isDab = mode == Mode.DAB
                val where = if (isDab) dabChannelAt(frequency)?.let { "Canal ${it.name} · ${mhz(it.hz)}" } ?: mhz(frequency) else khz(frequency)
                Text(
                    listOfNotNull(if (isDab) "DAB+" else "DRM", where, d.ensemble.ifBlank { null }).joinToString("  ·  "),
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                SignalBadge(playing, d)
                Spacer(Modifier.height(4.dp))
                val title = when {
                    !playing -> "Appuie sur ▶ pour écouter"
                    isDab -> d.playing ?: if (d.services.isEmpty()) "Recherche du multiplex…" else "Choisis une station"
                    else -> d.station.ifBlank { if (d.synced) "Station…" else "Recherche du signal DRM…" }
                }
                Text(title, style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (!isDab && d.info.isNotBlank()) {
                    Text(d.info, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (d.text.isNotBlank()) {
                    Text(d.text, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.primary, maxLines = 4, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

@Composable
private fun SignalBadge(playing: Boolean, d: DigitalState) {
    val (text, ok) = when {
        !playing -> "Arrêté" to false
        !d.synced -> "Pas de signal" to false
        d.kind == "dab" -> String.format(Locale.FRANCE, "Synchronisé · SNR %.0f dB", d.snr) to true
        else -> "Synchronisé" to true
    }
    Surface(
        color = if (ok) MaterialTheme.colorScheme.tertiaryContainer else MaterialTheme.colorScheme.surfaceContainerHighest,
        shape = RoundedCornerShape(50),
    ) {
        Text(text, style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp))
    }
}

/** Right side in the digital space: DAB+ channels and stations, or DRM frequencies */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DigitalPanel(
    mode: Mode?,
    frequency: Double,
    d: DigitalState,
    found: Map<String, String>,
    scan: Pair<Int, Int>?,
    volume: Float,
    onKind: (Mode) -> Unit,
    onTune: (Double) -> Unit,
    onService: (DabService) -> Unit,
    onScan: () -> Unit,
    onVolume: (Float) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            listOf(Mode.DAB, Mode.DRM).forEachIndexed { i, m ->
                SegmentedButton(
                    selected = mode == m,
                    onClick = { onKind(m) },
                    shape = SegmentedButtonDefaults.itemShape(i, 2),
                ) { Text(if (m == Mode.DAB) "DAB+" else "DRM") }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Filled.VolumeUp, "Volume", Modifier.size(20.dp))
            Slider(volume, onVolume, Modifier.padding(start = 8.dp))
        }
        if (mode == Mode.DAB) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Multiplex", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                FilledTonalButton(onClick = onScan) {
                    Icon(if (scan != null) Icons.Default.Stop else Icons.Default.Search, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(if (scan != null) "Arrêter" else "Rechercher")
                }
            }
            scan?.let { (i, n) ->
                Text("Canal ${DAB_CHANNELS[i].name} (${i + 1} / $n)", style = MaterialTheme.typography.bodySmall)
                LinearProgressIndicator(progress = { (i + 1f) / n }, modifier = Modifier.fillMaxWidth())
            }
            val here = dabChannelAt(frequency)?.name
            if (found.isNotEmpty()) {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    found.forEach { (ch, name) ->
                        val c = DAB_CHANNELS.firstOrNull { it.name == ch } ?: return@forEach
                        FilterChip(selected = here == ch, onClick = { onTune(c.hz) }, label = { Text("$ch · $name", maxLines = 1) })
                    }
                }
            }
            var all by rememberSaveable { mutableStateOf(found.isEmpty()) }
            TextButton(onClick = { all = !all }) { Text(if (all) "Masquer les canaux" else "Tous les canaux (5A à 13F)") }
            if (all) {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    DAB_CHANNELS.forEach { c ->
                        FilterChip(selected = here == c.name, onClick = { onTune(c.hz) }, label = { Text(c.name) })
                    }
                }
            }
            Text("Stations", style = MaterialTheme.typography.titleSmall)
            if (d.services.isEmpty()) {
                Text(
                    if (d.synced) "Lecture de la liste…" else "Pas de multiplex sur ce canal",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            d.services.sortedBy { it.label.lowercase() }.forEach { s ->
                val current = d.playing == s.label
                Surface(
                    color = if (current) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).clickable(enabled = s.dabPlus) { onService(s) },
                ) {
                    Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                        Text(s.label, style = MaterialTheme.typography.bodyLarge, fontWeight = if (current) FontWeight.Bold else FontWeight.Normal)
                        if (!s.dabPlus) {
                            Text("DAB (MP2) : pas encore pris en charge", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
        } else {
            Text("Fréquences", style = MaterialTheme.typography.titleSmall)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                DRM_PRESETS.forEach { p ->
                    FilterChip(
                        selected = kotlin.math.abs(frequency - p.hz) < 500,
                        onClick = { onTune(p.hz) },
                        label = { Text("${p.name} · ${khz(p.hz)}") },
                    )
                }
            }
            Text(
                "Ou tape une fréquence en haut. Il faut un récepteur HF (ou un KiwiSDR) et un débit modeste (48 à 192 kS/s).",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (d.status.isNotBlank()) {
            Text(d.status, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

// ---------------------------------------------------------------------------------- LRPT

/** Left side in the LRPT space: the picture as it comes down */
@Composable
fun LrptImagePanel(image: ImageBitmap?, l: LrptState, playing: Boolean, modifier: Modifier = Modifier) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLowest, shape = RoundedCornerShape(20.dp), modifier = modifier) {
        if (image != null) {
            Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState(), reverseScrolling = true), contentAlignment = Alignment.TopCenter) {
                Image(image, "Image Meteor", contentScale = ContentScale.FillWidth, modifier = Modifier.fillMaxWidth())
            }
        } else {
            Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(Icons.Default.SatelliteAlt, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(12.dp))
                Text(
                    when {
                        !playing -> "Appuie sur ▶ au début d'un passage"
                        l.locked -> "Signal accroché, l'image arrive…"
                        else -> "En attente du satellite…"
                    },
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    "Antenne dehors (QFH ou V-dipôle), passage au-dessus de 20° d'élévation.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/** Right side in the LRPT space: downlink, decoding state, saving */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun LrptPanel(frequency: Double, l: LrptState, onTune: (Double) -> Unit, onOqpsk: (Boolean) -> Unit, onSave: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("Satellite", style = MaterialTheme.typography.titleSmall)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            LRPT_SATELLITES.forEach { s ->
                FilterChip(selected = kotlin.math.abs(frequency - s.hz) < 5_000, onClick = { onTune(s.hz) }, label = { Text(s.name) })
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("OQPSK", style = MaterialTheme.typography.bodyLarge)
                Text("Meteor-M N2-3 et N2-4 ; décoché : QPSK", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Switch(l.oqpsk, onOqpsk)
        }
        Text("Réception", style = MaterialTheme.typography.titleSmall)
        Stat("État", if (!l.active) "—" else if (l.locked) "Trames accrochées" else "Recherche")
        Stat("Porteuse", String.format(Locale.FRANCE, "%+.0f Hz", l.carrier))
        Stat("Trames", "${l.framesOk} bonnes · ${l.framesBad} ratées")
        Stat("Image", "${l.lines} lignes · canaux ${l.channels.joinToString().ifBlank { "—" }}")
        FilledTonalButton(onClick = onSave, enabled = l.packets > 0, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Default.Save, null, Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text("Enregistrer les images")
        }
    }
}

@Composable
private fun Stat(name: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(name, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}

/** Top cap of the right panel outside the SDR space: the space's name */
@Composable
fun SpaceTitle(space: AppSpace, modifier: Modifier = Modifier) {
    Row(modifier.padding(horizontal = 18.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(space.icon, null, tint = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.width(10.dp))
        Text(space.label, style = MaterialTheme.typography.titleLarge)
    }
}
