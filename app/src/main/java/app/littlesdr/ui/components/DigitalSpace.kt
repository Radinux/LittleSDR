package app.littlesdr.ui.components

import app.littlesdr.engine.SmAction
import app.littlesdr.engine.SmWidget

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.basicMarquee
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Router
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SignalCellularAlt
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material.icons.rounded.Radio
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.produceState
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.max
import androidx.compose.ui.unit.min
import app.littlesdr.engine.DAB_CHANNELS
import app.littlesdr.engine.DRM_PRESETS
import app.littlesdr.engine.DabChannel
import app.littlesdr.engine.DRM_BANDS
import app.littlesdr.engine.DabService
import app.littlesdr.engine.HfBand
import app.littlesdr.engine.hfBandAt
import app.littlesdr.engine.DigitalState
import app.littlesdr.engine.dabChannelAt
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt

// DAB+ and DRM spaces, laid out like a car head unit: the "now playing" card on the left
// (the whole height in DAB+, under a spectrum strip in DRM), zapping on the right.
// DAB+ zaps multiplexes and stations; DRM is shortwave, so it zaps HF bands and frequencies.

private fun mhz(hz: Double) = String.format(Locale.FRANCE, "%.3f MHz", hz / 1e6)
private fun khz(hz: Double) = String.format(Locale.FRANCE, "%,.0f kHz", hz / 1e3)

// ================================================================================ left: player

/**
 * Left side of the digital space: RF status on top, slideshow + station + programme text in
 * the middle, transport and volume at the bottom, all inside one card.
 */
@Composable
fun DigitalNowPlaying(
    d: DigitalState,
    slide: ImageBitmap?,
    isDab: Boolean,
    frequency: Double,
    playing: Boolean,
    playEnabled: Boolean,
    volume: Float,
    onPlay: () -> Unit,
    onVolume: (Float) -> Unit,
    onAudioSettings: () -> Unit,
    cornerRadius: Dp,
    modifier: Modifier = Modifier,
) {
    // Enough inset that nothing touches the rounded corners
    val inset = max(16.dp, cornerRadius * 0.35f)
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(cornerRadius), modifier = modifier) {
        Column(Modifier.fillMaxSize().padding(horizontal = inset, vertical = 12.dp)) {
            RfStatusBar(d, isDab, frequency, playing, Modifier.fillMaxWidth().padding(horizontal = 4.dp))
            Crossfade(playing, label = "nowPlaying", modifier = Modifier.weight(1f).fillMaxWidth().padding(vertical = 12.dp)) { p ->
                if (p) PlayingContent(d, slide, isDab) else IdleContent(isDab)
            }
            PlayerControls(playing, playEnabled, volume, onPlay, onVolume, onAudioSettings)
        }
    }
}

/** One line: band, channel, frequency, multiplex; the lock badge on the right */
@Composable
private fun RfStatusBar(d: DigitalState, isDab: Boolean, frequency: Double, playing: Boolean, modifier: Modifier = Modifier) {
    val where = if (isDab) dabChannelAt(frequency)?.let { "Canal ${it.name} · ${mhz(it.hz)}" } ?: mhz(frequency)
    else listOfNotNull(khz(frequency), hfBandAt(frequency)?.name).joinToString("  ·  ")
    val line = listOfNotNull(if (isDab) "DAB+" else "DRM", where, d.ensemble.ifBlank { null }).joinToString("  ·  ")
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        // Scrolls instead of being cut when the multiplex name is long
        Text(
            line,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            modifier = Modifier.weight(1f).basicMarquee(),
        )
        Spacer(Modifier.width(10.dp))
        SignalBadge(playing, d)
    }
}

@Composable
private fun SignalBadge(playing: Boolean, d: DigitalState) {
    val cs = MaterialTheme.colorScheme
    val (text, ok) = when {
        !playing -> "Arrêté" to false
        !d.synced -> "Pas de signal" to false
        d.kind == "dab" -> String.format(Locale.FRANCE, "Synchronisé · SNR %.0f dB", d.snr) to true
        else -> "Synchronisé" to true
    }
    Surface(color = if (ok) cs.tertiaryContainer else cs.surfaceContainerHighest, shape = CircleShape) {
        Row(Modifier.padding(start = 8.dp, end = 10.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(7.dp).clip(CircleShape).background(if (ok) cs.onTertiaryContainer else cs.outline))
            Spacer(Modifier.width(6.dp))
            Text(text, style = MaterialTheme.typography.labelMedium, maxLines = 1, softWrap = false)
        }
    }
}

/** Logo of a station from the internet (null while loading, unknown, or not allowed now) */
@Composable
internal fun stationLogo(name: String?): ImageBitmap? {
    val on by app.littlesdr.data.StationLogos.enabled.collectAsState()
    val mobile by app.littlesdr.data.StationLogos.mobileData.collectAsState()
    val logo by produceState<ImageBitmap?>(null, name, on, mobile) {
        value = if (name.isNullOrBlank() || !on) null else app.littlesdr.data.StationLogos.get(name)
    }
    return logo
}

/** Slideshow on the left, station / programme text / audio on the right */
@Composable
private fun PlayingContent(d: DigitalState, slide: ImageBitmap?, isDab: Boolean) {
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.CenterStart) {
        // Square artwork as tall as the room allows, but never more than ~40 % of the width
        val art = min(maxHeight, maxWidth * 0.42f)
        val compact = maxHeight < 170.dp
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(if (compact) 16.dp else 24.dp)) {
            Artwork(slide, if (isDab) d.playing else d.station.ifBlank { null }, Modifier.size(art))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(if (compact) 4.dp else 8.dp)) {
                val title = when {
                    isDab -> d.playing ?: if (d.services.isEmpty()) "Recherche du multiplex…" else "Choisis une station"
                    else -> d.station.ifBlank { if (d.synced) "Station…" else "Recherche du signal DRM…" }
                }
                val onAir = if (isDab) d.playing != null else d.station.isNotBlank()
                Text(
                    title,
                    style = if (!onAir) MaterialTheme.typography.headlineSmall
                    else if (compact) MaterialTheme.typography.headlineMedium else MaterialTheme.typography.displaySmall,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    modifier = Modifier.basicMarquee(),
                )
                if (d.text.isNotBlank()) {
                    Text(
                        d.text,
                        style = if (compact) MaterialTheme.typography.titleSmall else MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.primary,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                val audio = if (isDab) listOfNotNull("DAB+", d.bitrate.takeIf { it > 0 }?.let { "$it kbit/s" }).joinToString(" · ") else d.info
                if (onAir && audio.isNotBlank()) {
                    Text(
                        audio,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        modifier = Modifier.basicMarquee(),
                    )
                }
            }
        }
    }
}

/** MOT slideshow in a rounded square; a radio icon until a picture arrives */
@Composable
private fun Artwork(slide: ImageBitmap?, station: String?, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    // The station's logo (internet) until its own slideshow arrives
    val logo = stationLogo(station)
    Box(
        modifier.clip(RoundedCornerShape(16.dp)).background(cs.surfaceContainerHighest),
        contentAlignment = Alignment.Center,
    ) {
        Crossfade(slide ?: logo, label = "slide") { s ->
            if (s != null) {
                // Slides are 4:3: fit, so text on them is never cropped
                Image(s, if (s === slide) "Diaporama de la station" else "Logo de la station", contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxSize().padding(if (s === slide) 0.dp else 12.dp))
            } else {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Icon(Icons.Rounded.Radio, null, Modifier.fillMaxSize(0.42f), tint = cs.primary.copy(alpha = 0.7f))
                }
            }
        }
    }
}

/** Stopped: a quiet hint, icons only */
@Composable
private fun IdleContent(isDab: Boolean) {
    Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(Icons.Rounded.Radio, null, Modifier.size(56.dp), tint = MaterialTheme.colorScheme.primary.copy(alpha = 0.7f))
        Spacer(Modifier.height(12.dp))
        Text("Radio à l'arrêt", style = MaterialTheme.typography.titleLarge, textAlign = TextAlign.Center)
        Spacer(Modifier.height(4.dp))
        Text(
            if (isDab) "Lance la lecture pour capter le multiplex" else "Lance la lecture pour chercher la station DRM",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

/** Transport bar inside the card: play/stop, mute, volume, audio settings */
@Composable
private fun PlayerControls(
    playing: Boolean,
    playEnabled: Boolean,
    volume: Float,
    onPlay: () -> Unit,
    onVolume: (Float) -> Unit,
    onAudioSettings: () -> Unit,
) {
    val cs = MaterialTheme.colorScheme
    // Local value while dragging: the engine's echo must not make the thumb jump back
    val interaction = remember { MutableInteractionSource() }
    val dragging by interaction.collectIsDraggedAsState()
    var local by remember { mutableFloatStateOf(volume) }
    LaunchedEffect(volume, dragging) { if (!dragging) local = volume }
    // Mute keeps the previous level so un-muting restores it
    var lastVolume by remember { mutableFloatStateOf(if (volume > 0.001f) volume else 0.8f) }
    val muted = local <= 0.001f

    Surface(color = cs.surfaceContainerHigh, shape = CircleShape, modifier = Modifier.fillMaxWidth()) {
        Row(Modifier.padding(6.dp), verticalAlignment = Alignment.CenterVertically) {
            FilledIconButton(
                onClick = onPlay,
                enabled = playEnabled,
                modifier = Modifier.size(52.dp),
                colors = if (playing) IconButtonDefaults.filledIconButtonColors(
                    containerColor = cs.secondaryContainer,
                    contentColor = cs.onSecondaryContainer,
                ) else IconButtonDefaults.filledIconButtonColors(),
            ) {
                AnimatedContent(playing, label = "play") { p ->
                    Icon(if (p) Icons.Default.Stop else Icons.Default.PlayArrow, if (p) "Arrêter" else "Écouter", Modifier.size(28.dp))
                }
            }
            Spacer(Modifier.width(8.dp))
            IconButton(onClick = {
                if (muted) onVolume(lastVolume) else { lastVolume = local; onVolume(0f) }
            }) {
                Icon(
                    if (muted) Icons.AutoMirrored.Filled.VolumeOff else Icons.AutoMirrored.Filled.VolumeUp,
                    if (muted) "Rétablir le son" else "Couper le son",
                )
            }
            Slider(
                value = local,
                onValueChange = { local = it; onVolume(it) },
                interactionSource = interaction,
                modifier = Modifier.weight(1f),
            )
            Text(
                "${(local * 100).roundToInt()} %",
                style = MaterialTheme.typography.labelMedium,
                color = cs.onSurfaceVariant,
                textAlign = TextAlign.End,
                maxLines = 1,
                modifier = Modifier.width(44.dp),
            )
            IconButton(onClick = onAudioSettings) { Icon(Icons.Default.Tune, "Réglages audio") }
        }
    }
}

// ================================================================================ right: zapping

/** Top of the right panel: receiver badge (opens the source settings), settings */
@Composable
fun DigitalHeader(
    source: String,
    usb: Boolean,
    sourceOpen: Boolean,
    onSource: () -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val cs = MaterialTheme.colorScheme
    Row(modifier, horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
        Surface(
            onClick = onSource,
            shape = CircleShape,
            color = if (sourceOpen) cs.secondaryContainer else cs.surfaceContainerHigh,
            modifier = Modifier.weight(1f, fill = false),
        ) {
            Row(Modifier.padding(horizontal = 10.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(if (usb) Icons.Default.Usb else Icons.Default.Router, null, Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(
                    source.ifBlank { "Source" },
                    style = MaterialTheme.typography.labelLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        IconButton(onClick = onSettings) { Icon(Icons.Default.Settings, "Réglages") }
    }
}

/** DAB+ below the header: one-line channel selector, then the stations take the rest */
@Composable
fun DabTuner(
    frequency: Double,
    d: DigitalState,
    found: Map<String, String>,
    scan: Pair<Int, Int>?,
    onTune: (Double) -> Unit,
    onService: (DabService) -> Unit,
    onScan: () -> Unit,
    af: Boolean = true,
    onAf: (Boolean) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    var grid by rememberSaveable { mutableStateOf(false) }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        ChannelSelector(frequency, d, found, scan, onTune, onScan, onOpenGrid = { grid = true })
        scan?.let { (i, n) ->
            LinearProgressIndicator(progress = { (i + 1f) / n }, modifier = Modifier.fillMaxWidth())
        }
        // Service following: when the multiplex is lost, look for the station elsewhere
        FilterChip(
            selected = af,
            onClick = { onAf(!af) },
            label = { Text(if (d.af.isNotEmpty()) "AF · suivi de station (${d.af.size} fréq.)" else "AF · suivi de station") },
        )
        StationList(d, onService, Modifier.weight(1f).fillMaxWidth())
        StatusLine(d.status)
    }
    if (grid) {
        ChannelGridSheet(frequency, found, onPick = { onTune(it.hz); grid = false }, onDismiss = { grid = false })
    }
}

/** Decoder status, small, at the foot of the right panel */
@Composable
private fun StatusLine(status: String) {
    if (status.isBlank()) return
    Text(
        status,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 2,
        overflow = TextOverflow.Ellipsis,
    )
}

/**
 * Next / previous channel. Once a scan has found multiplexes the arrows jump between those
 * (that is what zapping means); before that they walk band III channel by channel.
 */
internal fun stepChannel(frequency: Double, found: Map<String, String>, dir: Int): DabChannel {
    val pool = DAB_CHANNELS.filter { it.name in found }.ifEmpty { DAB_CHANNELS }
    return if (dir > 0) pool.firstOrNull { it.hz > frequency + 1_000 } ?: pool.first()
    else pool.lastOrNull { it.hz < frequency - 1_000 } ?: pool.last()
}

/** `◀  11D · multiplex  ▶` on a single line; the middle opens the full channel grid */
@Composable
private fun ChannelSelector(
    frequency: Double,
    d: DigitalState,
    found: Map<String, String>,
    scan: Pair<Int, Int>?,
    onTune: (Double) -> Unit,
    onScan: () -> Unit,
    onOpenGrid: () -> Unit,
) {
    val cs = MaterialTheme.colorScheme
    val here = dabChannelAt(frequency)
    val jump = if (found.isEmpty()) "Canal" else "Multiplex"
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        FilledTonalIconButton(onClick = { onTune(stepChannel(frequency, found, -1).hz) }) {
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowLeft, "$jump précédent")
        }
        Surface(
            onClick = onOpenGrid,
            shape = RoundedCornerShape(14.dp),
            color = cs.surfaceContainerHigh,
            modifier = Modifier.weight(1f).padding(horizontal = 6.dp).height(52.dp),
        ) {
            Row(Modifier.padding(start = 12.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    val (title, subtitle) = when {
                        scan != null -> "Recherche ${DAB_CHANNELS[scan.first].name}" to "${scan.first + 1} / ${scan.second}"
                        here != null -> here.name to (found[here.name] ?: d.ensemble.ifBlank { null } ?: mhz(here.hz))
                        else -> "—" to mhz(frequency)
                    }
                    Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, color = cs.primary, maxLines = 1)
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.labelMedium,
                        color = cs.onSurfaceVariant,
                        maxLines = 1,
                        modifier = Modifier.basicMarquee(),
                    )
                }
                Icon(Icons.Default.GridView, "Tous les canaux", Modifier.size(20.dp), tint = cs.onSurfaceVariant)
            }
        }
        FilledTonalIconButton(onClick = { onTune(stepChannel(frequency, found, 1).hz) }) {
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, "$jump suivant")
        }
        IconButton(onClick = onScan) {
            Icon(
                if (scan != null) Icons.Default.Stop else Icons.Default.Search,
                if (scan != null) "Arrêter la recherche" else "Rechercher les multiplex",
            )
        }
    }
}

/** Stations of the multiplex: all the remaining height, the one on air in amber */
@Composable
private fun StationList(d: DigitalState, onService: (DabService) -> Unit, modifier: Modifier = Modifier) {
    val services = remember(d.services) { d.services.sortedBy { it.label.lowercase() } }
    if (services.isEmpty()) {
        Column(modifier, verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(Icons.Default.SignalCellularAlt, null, Modifier.size(32.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(8.dp))
            Text(
                if (d.synced) "Lecture de la liste des stations…" else "Pas de multiplex sur ce canal",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
        return
    }
    val listState = rememberLazyListState()
    val current = services.indexOfFirst { it.label == d.playing }
    // Keep the station on air in view (after a zap from elsewhere, or a fresh list)
    LaunchedEffect(current) {
        if (current >= 0 && listState.layoutInfo.visibleItemsInfo.none { it.index == current }) listState.animateScrollToItem(current)
    }
    LazyColumn(modifier, state = listState, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        items(services) { s -> StationRow(s, s.label == d.playing) { onService(s) } }
    }
}

@Composable
private fun StationRow(s: DabService, current: Boolean, onClick: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        enabled = s.dabPlus,
        shape = RoundedCornerShape(12.dp),
        color = if (current) cs.primary.copy(alpha = 0.16f) else cs.surfaceContainerHigh,
        border = if (current) BorderStroke(1.5.dp, cs.primary) else null,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(Modifier.padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            stationLogo(s.label)?.let { logo ->
                Image(logo, null, contentScale = ContentScale.Fit, modifier = Modifier.size(32.dp).clip(RoundedCornerShape(6.dp)))
                Spacer(Modifier.width(12.dp))
            }
            Column(Modifier.weight(1f)) {
                Text(
                    s.label,
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = if (current) FontWeight.Bold else FontWeight.Normal,
                    color = when {
                        current -> cs.primary
                        !s.dabPlus -> cs.onSurfaceVariant
                        else -> cs.onSurface
                    },
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (!s.dabPlus) {
                    Text("DAB (MP2) : pas encore pris en charge", style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
                }
            }
            if (current) Icon(Icons.Default.GraphicEq, "À l'antenne", Modifier.size(20.dp), tint = cs.primary)
        }
    }
}

/** Every band III channel, one row per block, the multiplexes found marked */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
internal fun ChannelGridSheet(frequency: Double, found: Map<String, String>, onPick: (DabChannel) -> Unit, onDismiss: () -> Unit) {
    val here = dabChannelAt(frequency)?.name
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(start = 24.dp, end = 24.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (found.isNotEmpty()) {
                Text("Multiplex trouvés", style = MaterialTheme.typography.titleSmall)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    DAB_CHANNELS.filter { it.name in found }.forEach { c ->
                        FilterChip(
                            selected = here == c.name,
                            onClick = { onPick(c) },
                            label = { Text("${c.name} · ${found[c.name]}", maxLines = 1) },
                            leadingIcon = { Icon(Icons.Rounded.Radio, null, Modifier.size(16.dp)) },
                        )
                    }
                }
                Spacer(Modifier.height(4.dp))
            }
            Text("Tous les canaux", style = MaterialTheme.typography.titleSmall)
            DAB_CHANNELS.groupBy { it.name.dropLast(1) }.forEach { (block, channels) ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        block,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.width(32.dp),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        channels.forEach { c ->
                            FilterChip(
                                selected = here == c.name,
                                onClick = { onPick(c) },
                                label = { Text(c.name) },
                                leadingIcon = if (c.name in found) {
                                    { Icon(Icons.Rounded.Radio, "Multiplex trouvé", Modifier.size(16.dp)) }
                                } else null,
                            )
                        }
                    }
                }
            }
        }
    }
}


// ================================================================================ DRM (shortwave)

/**
 * DRM below the header: the typed frequency, a one-line HF band selector, then the known
 * frequencies. The spectrum strip on the left lets you tap a DRM block you see in the band.
 */
@Composable
fun DrmTuner(
    frequency: Double,
    d: DigitalState,
    onTune: (Double) -> Unit,
    controls: List<SmWidget>,
    onControl: (SmAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    var bands by rememberSaveable { mutableStateOf(false) }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        FrequencyDisplay(frequency, onTune, Modifier.fillMaxWidth())
        BandSelector(frequency, onTune, onOpenGrid = { bands = true })
        // Gain and bias-tee (active loop / preamp) without going into the source settings
        if (controls.isNotEmpty()) GainControls(controls, onControl)
        Text("Fréquences DRM connues", style = MaterialTheme.typography.titleSmall)
        DrmPresetList(frequency, onTune, Modifier.weight(1f).fillMaxWidth())
        StatusLine(d.status)
    }
    if (bands) {
        BandGridSheet(frequency, onPick = { onTune(it); bands = false }, onDismiss = { bands = false })
    }
}

/** Where a band jump lands: the first known DRM frequency in it, else the band centre */
private fun landing(b: HfBand): Double = DRM_PRESETS.firstOrNull { it.hz in b }?.hz ?: b.centre

private fun stepBand(frequency: Double, dir: Int): HfBand =
    if (dir > 0) DRM_BANDS.firstOrNull { it.lo > frequency } ?: DRM_BANDS.first()
    else DRM_BANDS.lastOrNull { it.hi < frequency } ?: DRM_BANDS.last()

/** `◀  25 m · 11,600 – 12,100 MHz  ▶` on one line; the middle opens all bands */
@Composable
private fun BandSelector(frequency: Double, onTune: (Double) -> Unit, onOpenGrid: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    val band = hfBandAt(frequency)
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        FilledTonalIconButton(onClick = { onTune(landing(stepBand(frequency, -1))) }) {
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowLeft, "Bande précédente")
        }
        Surface(
            onClick = onOpenGrid,
            shape = RoundedCornerShape(14.dp),
            color = cs.surfaceContainerHigh,
            modifier = Modifier.weight(1f).padding(horizontal = 6.dp).height(52.dp),
        ) {
            Row(Modifier.padding(start = 12.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(
                        band?.name ?: "Hors bande",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                        color = if (band != null) cs.primary else cs.onSurfaceVariant,
                        maxLines = 1,
                    )
                    Text(
                        band?.let { "${khz(it.lo)} – ${khz(it.hi)}" } ?: "Bandes de radiodiffusion",
                        style = MaterialTheme.typography.labelMedium,
                        color = cs.onSurfaceVariant,
                        maxLines = 1,
                        modifier = Modifier.basicMarquee(),
                    )
                }
                Icon(Icons.Default.GridView, "Toutes les bandes", Modifier.size(20.dp), tint = cs.onSurfaceVariant)
            }
        }
        FilledTonalIconButton(onClick = { onTune(landing(stepBand(frequency, 1))) }) {
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, "Bande suivante")
        }
    }
}

@Composable
private fun DrmPresetList(frequency: Double, onTune: (Double) -> Unit, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    val listState = rememberLazyListState()
    val current = DRM_PRESETS.indexOfFirst { abs(frequency - it.hz) < 500 }
    LaunchedEffect(current) {
        if (current >= 0 && listState.layoutInfo.visibleItemsInfo.none { it.index == current }) listState.animateScrollToItem(current)
    }
    LazyColumn(modifier, state = listState, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        items(DRM_PRESETS) { p ->
            val on = abs(frequency - p.hz) < 500
            Surface(
                onClick = { onTune(p.hz) },
                shape = RoundedCornerShape(12.dp),
                color = if (on) cs.primary.copy(alpha = 0.16f) else cs.surfaceContainerHigh,
                border = if (on) BorderStroke(1.5.dp, cs.primary) else null,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(Modifier.padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(
                            p.name,
                            style = MaterialTheme.typography.bodyLarge,
                            fontWeight = if (on) FontWeight.Bold else FontWeight.Normal,
                            color = if (on) cs.primary else cs.onSurface,
                            maxLines = 1,
                        )
                        hfBandAt(p.hz)?.let {
                            Text(it.name, style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
                        }
                    }
                    Text(khz(p.hz), style = MaterialTheme.typography.bodyMedium, color = if (on) cs.primary else cs.onSurfaceVariant, maxLines = 1)
                }
            }
        }
        item {
            Text(
                "Il faut un récepteur HF (ou un KiwiSDR) et un débit modeste (48 à 192 kS/s).",
                style = MaterialTheme.typography.bodySmall,
                color = cs.onSurfaceVariant,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}

/** All broadcast bands, GO to 11 m, with the DRM frequencies known in each */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
private fun BandGridSheet(frequency: Double, onPick: (Double) -> Unit, onDismiss: () -> Unit) {
    val here = hfBandAt(frequency)
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(start = 24.dp, end = 24.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Bandes de radiodiffusion", style = MaterialTheme.typography.titleSmall)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                DRM_BANDS.forEach { b ->
                    val known = DRM_PRESETS.any { it.hz in b }
                    FilterChip(
                        selected = here == b,
                        onClick = { onPick(landing(b)) },
                        label = { Text(b.name) },
                        leadingIcon = if (known) {
                            { Icon(Icons.Rounded.Radio, "Fréquence DRM connue", Modifier.size(16.dp)) }
                        } else null,
                    )
                }
            }
        }
    }
}
