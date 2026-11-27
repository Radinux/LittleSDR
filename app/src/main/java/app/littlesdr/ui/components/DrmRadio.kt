package app.littlesdr.ui.components

import app.littlesdr.ui.theme.pillBorder
import androidx.compose.runtime.produceState
import kotlinx.coroutines.delay
import java.time.ZoneOffset
import java.time.ZonedDateTime
import app.littlesdr.data.DrmBroadcast
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
import app.littlesdr.ui.theme.FilledIconButton
import app.littlesdr.ui.theme.FilledTonalIconButton
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
    schedule: List<DrmBroadcast>,
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
    // The clock for "on air now": the schedule changes on the hour, a minute is precise enough
    val now by produceState(ZonedDateTime.now(ZoneOffset.UTC)) {
        while (true) {
            delay(30_000)
            value = ZonedDateTime.now(ZoneOffset.UTC)
        }
    }
    // What is scheduled on this frequency now: the schedule of DRM broadcasts, else the EiBi one
    val here = schedule.firstOrNull { abs(it.hz - frequency) < 1_000 && it.onAir(now) }
    val scheduled = here?.let { listOfNotNull(it.name, it.audience.ifBlank { null }, it.site.ifBlank { null }).joinToString(" · ") }
        ?: eibi.firstOrNull()?.let { e ->
            listOfNotNull(e.station, listOfNotNull(e.languageText.ifBlank { null }, e.target.ifBlank { null }).joinToString(" → ").ifBlank { null }, e.country.ifBlank { null }).joinToString(" · ")
        }
    BoxWithConstraints(modifier) {
        val portrait = maxHeight > maxWidth
        val display = @Composable { m: Modifier ->
            DrmDisplay(d, slide, frequency, playing, playEnabled, source, usb, cornerRadius, scheduled, here?.name, onPlay, onStep, onSource, onSettings, m)
        }
        val tuning = @Composable { m: Modifier ->
            DrmTuning(frequency, space, onSpace, cornerRadius, heard, schedule, now, controls, spectrum, onTune, onForget, onControl, portrait, m)
        }
        if (portrait) {
            // Same split as the SDR screen: the space switcher falls at the same place
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                display(Modifier.fillMaxWidth().weight(0.42f))
                tuning(Modifier.fillMaxWidth().weight(0.58f))
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
    cornerRadius: Dp,
    scheduled: String?,
    scheduledName: String?,
    onPlay: () -> Unit,
    onStep: (Int) -> Unit,
    onSource: () -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier,
) {
    val cs = MaterialTheme.colorScheme
    var status by rememberSaveable { mutableStateOf(false) }
    Surface(color = cs.surfaceContainerLow, shape = RoundedCornerShape(cornerRadius), border = pillBorder(), modifier = modifier) {
        SlideBackdrop(slide.takeIf { playing })
        Column(Modifier.fillMaxSize().padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            // Top bar: receiver, signal, settings (the space switcher is on the tuning panel; the receiver's name gives way first)
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
            }
            // The display
            Row(Modifier.weight(1f).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(18.dp)) {
                DrmArtwork(slide, d.station.takeIf { playing && it.isNotBlank() } ?: scheduledName, Modifier.fillMaxHeight().aspectRatio(1f, matchHeightConstraintsFirst = true))
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
                    // What the schedule says is on this frequency now
                    scheduled?.let { line ->
                        Text(
                            line,
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
    space: AppSpace,
    onSpace: (AppSpace) -> Unit,
    cornerRadius: Dp,
    heard: List<DrmHeard>,
    schedule: List<DrmBroadcast>,
    now: ZonedDateTime,
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
    // The band around the frequency (a DRM block is a flat 10 kHz rectangle) as a slim strip
    // on top, the space switcher beside it at the place of the SDR one
    SpaceCappedPanel(space, onSpace, cornerRadius, modifier, head = { hm -> spectrum(hm) }) {
        Column(Modifier.fillMaxSize().padding(12.dp), verticalArrangement = Arrangement.spacedBy(if (portrait) 10.dp else 6.dp)) {
            BandSelector(frequency, onTune, onOpenGrid = { bands = true })
            // Fine tuning, 5 kHz steps (the broadcast raster)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                FilledTonalIconButton(onClick = { onTune(frequency - 5_000) }, modifier = Modifier.size(if (portrait) 48.dp else 40.dp)) { Icon(Icons.Default.Remove, "5 kHz plus bas") }
                Text(
                    khz(frequency),
                    style = if (portrait) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    modifier = Modifier.weight(1f),
                )
                FilledTonalIconButton(onClick = { onTune(frequency + 5_000) }, modifier = Modifier.size(if (portrait) 48.dp else 40.dp)) { Icon(Icons.Default.Add, "5 kHz plus haut") }
            }
            // On air now (Europe, Africa first: what can be heard here), later today, received here
            val onAir = schedule.filter { it.onAir(now) }.sortedWith(compareBy({ it.region != "region_1" }, { it.hz }))
            val later = schedule.mapNotNull { b -> b.startsIn(now)?.takeIf { !b.onAir(now) }?.let { b to it } }
                .sortedWith(compareBy({ it.first.region != "region_1" }, { it.second }))
            val local = java.time.ZonedDateTime.now()
            fun at(minutes: Int) = local.plusMinutes(minutes.toLong()).let { "à %02d:%02d".format(it.hour, it.minute) }
            LazyVerticalGrid(
                columns = GridCells.Adaptive(150.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.weight(1f).fillMaxWidth(),
            ) {
                item(span = { GridItemSpan(maxLineSpan) }) { SectionTitle("À l'antenne maintenant") }
                if (onAir.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }) {
                    Text("Aucune émission DRM prévue en ce moment.", style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
                }
                items(onAir, key = { "a${it.hz}${it.name}" }) { b ->
                    DrmTile(b.name, b.hz, abs(frequency - b.hz) < 1_000, Modifier.combinedClickable(onClick = { onTune(b.hz) }), b.audience)
                }
                if (later.isNotEmpty()) {
                    item(span = { GridItemSpan(maxLineSpan) }) { SectionTitle("Plus tard") }
                    items(later.take(12), key = { "l${it.first.hz}${it.first.name}" }) { (b, inMin) ->
                        DrmTile(b.name, b.hz, abs(frequency - b.hz) < 1_000, Modifier.combinedClickable(onClick = { onTune(b.hz) }), "${at(inMin)} · ${b.audience}")
                    }
                }
                if (heard.isNotEmpty()) {
                    item(span = { GridItemSpan(maxLineSpan) }) { SectionTitle("Stations reçues ici") }
                    items(heard, key = { "h${it.hz}" }) { h ->
                        DrmTile(h.name, h.hz, abs(frequency - h.hz) < 1_000, Modifier.combinedClickable(onClick = { onTune(h.hz) }, onLongClick = { onForget(h) }), "appui long : oublier")
                    }
                }
            }
            // Gain, AGC, bias-tee: in the receiver window (source chip), not repeated here
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
private fun DrmTile(name: String, hz: Double, current: Boolean, clicks: Modifier, detail: String = "") {
    val cs = MaterialTheme.colorScheme
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = if (current) cs.primaryContainer else cs.surfaceContainerHigh,
        border = if (current) BorderStroke(2.dp, cs.primary) else null,
        modifier = Modifier.fillMaxWidth().height(if (detail.isBlank()) 76.dp else 92.dp).clip(RoundedCornerShape(16.dp)).then(clicks),
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
                if (detail.isNotBlank()) Text(
                    detail,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (current) cs.onPrimaryContainer else cs.primary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}
