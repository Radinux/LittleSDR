package app.littlesdr.ui.components

import androidx.compose.animation.Crossfade
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.basicMarquee
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
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
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Router
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material.icons.rounded.Public
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.littlesdr.data.EiBiEntry
import app.littlesdr.engine.DRM_PRESETS
import app.littlesdr.engine.DigitalState
import app.littlesdr.engine.DrmHeard
import app.littlesdr.engine.SmAction
import app.littlesdr.engine.SmWidget
import app.littlesdr.engine.hfBandAt
import kotlin.math.abs

/**
 * DRM, as a shortwave receiver: a display card (station, its text, what the schedule says is
 * on this frequency, big ⏮ ▶ ⏭ through the stations) and a tuning card (the band around the
 * frequency, band and fine tuning, the stations received here and the known DRM frequencies
 * as tiles, the gain). Side by side in landscape, one above the other upright.
 */
@Composable
fun DrmRadioScreen(
    d: DigitalState,
    slide: ImageBitmap?,
    frequency: Double,
    playing: Boolean,
    playEnabled: Boolean,
    source: String,
    usb: Boolean,
    space: AppSpace,
    cornerRadius: Dp,
    eibi: List<EiBiEntry>,
    heard: List<DrmHeard>,
    controls: List<SmWidget>,
    spectrum: @Composable (Modifier) -> Unit,
    onPlay: () -> Unit,
    onStep: (Int) -> Unit,
    onTune: (Double) -> Unit,
    onForget: (DrmHeard) -> Unit,
    onControl: (SmAction) -> Unit,
    onSource: () -> Unit,
    onSettings: () -> Unit,
    onSpace: (AppSpace) -> Unit,
    modifier: Modifier = Modifier,
) {
    BoxWithConstraints(modifier) {
        val portrait = maxHeight > maxWidth
        val display = @Composable { m: Modifier ->
            DrmDisplay(d, slide, frequency, playing, playEnabled, source, usb, space, cornerRadius, eibi.firstOrNull(), onPlay, onStep, onSource, onSettings, onSpace, m)
        }
        val tuning = @Composable { m: Modifier ->
            DrmTuning(frequency, cornerRadius, heard, controls, spectrum, onTune, onForget, onControl, portrait, m)
        }
        if (portrait) {
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                display(Modifier.fillMaxWidth().weight(0.44f))
                tuning(Modifier.fillMaxWidth().weight(0.56f))
            }
        } else {
            Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                display(Modifier.weight(1.1f).fillMaxHeight())
                tuning(Modifier.weight(1f).fillMaxHeight())
            }
        }
    }
}

@Composable
private fun DrmDisplay(
    d: DigitalState,
    slide: ImageBitmap?,
    frequency: Double,
    playing: Boolean,
    playEnabled: Boolean,
    source: String,
    usb: Boolean,
    space: AppSpace,
    cornerRadius: Dp,
    schedule: EiBiEntry?,
    onPlay: () -> Unit,
    onStep: (Int) -> Unit,
    onSource: () -> Unit,
    onSettings: () -> Unit,
    onSpace: (AppSpace) -> Unit,
    modifier: Modifier,
) {
    val cs = MaterialTheme.colorScheme
    var status by rememberSaveable { mutableStateOf(false) }
    Surface(color = cs.surfaceContainerLow, shape = RoundedCornerShape(cornerRadius), modifier = modifier) {
        SlideBackdrop(slide.takeIf { playing })
        Column(Modifier.fillMaxSize().padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            // Top bar: receiver, signal, settings, space (the receiver's name gives way first)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Surface(onClick = onSource, shape = RoundedCornerShape(50), color = cs.surfaceContainerHigh, modifier = Modifier.weight(1f, fill = false)) {
                    Row(Modifier.padding(horizontal = 10.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(if (usb) Icons.Default.Usb else Icons.Default.Router, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(source, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                Spacer(Modifier.weight(0.001f))
                SignalBars(playing, d, Modifier.clip(RoundedCornerShape(8.dp)).padding(4.dp), onClick = { status = true })
                IconButton(onClick = onSettings) { Icon(Icons.Default.Settings, "Réglages") }
                SpaceSwitcher(space = space, onSpace = onSpace, shape = CircleShape, modifier = Modifier.size(48.dp))
            }
            // The display
            Row(Modifier.weight(1f).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(18.dp)) {
                DrmArtwork(slide, d.station.takeIf { playing && it.isNotBlank() }, Modifier.fillMaxHeight().aspectRatio(1f, matchHeightConstraintsFirst = true))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        listOfNotNull(khz(frequency), hfBandAt(frequency)?.name).joinToString(" · "),
                        style = MaterialTheme.typography.titleMedium,
                        color = cs.onSurfaceVariant,
                        maxLines = 1,
                        modifier = Modifier.basicMarquee(),
                    )
                    val onAir = playing && d.station.isNotBlank()
                    if (onAir) {
                        Text(d.station, style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.Bold, maxLines = 1, modifier = Modifier.basicMarquee())
                    } else {
                        WordFitText(
                            when {
                                !playing -> "Radio à l'arrêt"
                                !d.synced -> "Recherche du signal DRM…"
                                else -> "Signal accroché…"
                            },
                            style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold),
                            maxLines = 3,
                        )
                    }
                    if (playing && d.text.isNotBlank()) {
                        Text(d.text, style = MaterialTheme.typography.titleMedium, color = cs.primary, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                    // What the shortwave schedule says is on this frequency now
                    schedule?.let { e ->
                        Text(
                            listOf(
                                e.station.takeIf { !onAir },
                                listOfNotNull(e.languageText.ifBlank { null }, e.target.ifBlank { null }).joinToString(" → ").ifBlank { null },
                                e.country.ifBlank { null },
                            ).filterNotNull().joinToString(" · "),
                            style = MaterialTheme.typography.labelLarge,
                            color = cs.onSurfaceVariant,
                            maxLines = 1,
                            modifier = Modifier.basicMarquee(),
                        )
                    }
                    if (onAir && d.info.isNotBlank()) {
                        Text(d.info, style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant, maxLines = 1, modifier = Modifier.basicMarquee())
                    }
                }
            }
            // Transport: big targets
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterHorizontally)) {
                FilledTonalIconButton(onClick = { onStep(-1) }, modifier = Modifier.size(64.dp)) {
                    Icon(Icons.Default.SkipPrevious, "Station précédente", Modifier.size(36.dp))
                }
                FilledIconButton(
                    onClick = onPlay,
                    enabled = playEnabled,
                    modifier = Modifier.size(76.dp),
                    colors = if (playing) IconButtonDefaults.filledIconButtonColors(containerColor = cs.secondaryContainer, contentColor = cs.onSecondaryContainer)
                    else IconButtonDefaults.filledIconButtonColors(),
                ) {
                    Icon(if (playing) Icons.Default.Stop else Icons.Default.PlayArrow, if (playing) "Arrêter" else "Écouter", Modifier.size(44.dp))
                }
                FilledTonalIconButton(onClick = { onStep(1) }, modifier = Modifier.size(64.dp)) {
                    Icon(Icons.Default.SkipNext, "Station suivante", Modifier.size(36.dp))
                }
            }
        }
    }
    if (status) {
        AlertDialog(
            onDismissRequest = { status = false },
            confirmButton = { TextButton(onClick = { status = false }) { Text("OK") } },
            title = { Text("Réception DRM") },
            text = { Text(d.status.ifBlank { "Pas de réception en cours" }) },
        )
    }
}

/** Slideshow, else the station's logo, else a globe */
@Composable
private fun DrmArtwork(slide: ImageBitmap?, station: String?, modifier: Modifier) {
    val cs = MaterialTheme.colorScheme
    val logo = stationLogo(station)
    Box(modifier.clip(RoundedCornerShape(18.dp)).background(cs.surfaceContainerHighest), contentAlignment = Alignment.Center) {
        Crossfade(slide ?: logo, label = "art") { img ->
            when {
                img == null -> Icon(Icons.Rounded.Public, null, Modifier.fillMaxSize(0.45f), tint = cs.primary.copy(alpha = 0.7f))
                img === slide -> Image(img, "Diaporama de la station", contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                else -> LogoFill(img, description = "Logo de la station")
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun DrmTuning(
    frequency: Double,
    cornerRadius: Dp,
    heard: List<DrmHeard>,
    controls: List<SmWidget>,
    spectrum: @Composable (Modifier) -> Unit,
    onTune: (Double) -> Unit,
    onForget: (DrmHeard) -> Unit,
    onControl: (SmAction) -> Unit,
    portrait: Boolean,
    modifier: Modifier,
) {
    val cs = MaterialTheme.colorScheme
    var bands by rememberSaveable { mutableStateOf(false) }
    Surface(color = cs.surfaceContainerLow, shape = RoundedCornerShape(cornerRadius), modifier = modifier) {
        Column(Modifier.fillMaxSize().padding(12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            // The band around the frequency: a DRM block is a flat 10 kHz rectangle
            spectrum(Modifier.fillMaxWidth().height(if (portrait) 96.dp else 120.dp))
            BandSelector(frequency, onTune, onOpenGrid = { bands = true })
            // Fine tuning, 5 kHz steps (the broadcast raster)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                FilledTonalIconButton(onClick = { onTune(frequency - 5_000) }) { Icon(Icons.Default.Remove, "5 kHz plus bas") }
                Text(
                    khz(frequency),
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    modifier = Modifier.weight(1f),
                )
                FilledTonalIconButton(onClick = { onTune(frequency + 5_000) }) { Icon(Icons.Default.Add, "5 kHz plus haut") }
            }
            // Stations received here, then the known DRM frequencies
            val known = DRM_PRESETS.filter { p -> heard.none { abs(it.hz - p.hz) < 1_000 } }
            LazyVerticalGrid(
                columns = GridCells.Adaptive(150.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.weight(1f).fillMaxWidth(),
            ) {
                if (heard.isNotEmpty()) {
                    item(span = { GridItemSpan(maxLineSpan) }) { SectionTitle("Stations reçues ici") }
                    items(heard, key = { "h${it.hz}" }) { h ->
                        DrmTile(h.name, h.hz, abs(frequency - h.hz) < 1_000, Modifier.combinedClickable(onClick = { onTune(h.hz) }, onLongClick = { onForget(h) }))
                    }
                }
                if (known.isNotEmpty()) {
                    item(span = { GridItemSpan(maxLineSpan) }) { SectionTitle("Fréquences DRM connues") }
                    items(known, key = { "k${it.hz}" }) { p ->
                        DrmTile(p.name, p.hz, abs(frequency - p.hz) < 1_000, Modifier.combinedClickable(onClick = { onTune(p.hz) }))
                    }
                }
                if (heard.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }) {
                    Text(
                        "Les stations DRM que tu reçois s'ajoutent ici d'elles-mêmes. Appui long pour en retirer une.",
                        style = MaterialTheme.typography.bodySmall,
                        color = cs.onSurfaceVariant,
                    )
                }
            }
            // Gain and bias-tee (active loop, preamp) without going into the source settings
            if (controls.isNotEmpty()) GainControls(controls, onControl)
        }
    }
    if (bands) {
        BandGridSheet(frequency, onPick = { onTune(it); bands = false }, onDismiss = { bands = false })
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(top = 4.dp))
}

/** A DRM station or frequency: its logo when there is one, the name, the frequency and band */
@Composable
private fun DrmTile(name: String, hz: Double, current: Boolean, clicks: Modifier) {
    val cs = MaterialTheme.colorScheme
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = if (current) cs.primaryContainer else cs.surfaceContainerHigh,
        border = if (current) BorderStroke(2.dp, cs.primary) else null,
        modifier = Modifier.fillMaxWidth().height(76.dp).clip(RoundedCornerShape(16.dp)).then(clicks),
    ) {
        Row(Modifier.padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            val logo = stationLogo(name)
            if (logo != null) {
                Box(Modifier.size(44.dp).clip(RoundedCornerShape(10.dp))) { LogoFill(logo) }
                Spacer(Modifier.width(8.dp))
            }
            Column(Modifier.weight(1f)) {
                WordFitText(
                    name,
                    style = MaterialTheme.typography.titleSmall.copy(fontWeight = if (current) FontWeight.Bold else FontWeight.Medium),
                    color = if (current) cs.onPrimaryContainer else cs.onSurface,
                    maxLines = 2,
                )
                Text(
                    listOfNotNull(khz(hz), hfBandAt(hz)?.name).joinToString(" · "),
                    style = MaterialTheme.typography.labelMedium,
                    color = if (current) cs.onPrimaryContainer else cs.onSurfaceVariant,
                    maxLines = 1,
                )
            }
        }
    }
}
