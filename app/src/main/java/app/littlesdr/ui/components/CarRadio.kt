package app.littlesdr.ui.components

import androidx.compose.animation.Crossfade
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.basicMarquee
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Radar
import androidx.compose.material.icons.filled.Router
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material.icons.rounded.Radio
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.ImeAction
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
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.DAB_CHANNELS
import app.littlesdr.engine.DabService
import app.littlesdr.engine.DigitalState
import app.littlesdr.engine.dabChannelAt
import java.util.Locale

/**
 * DAB+ as a car radio: a big display (logo or slideshow, station name, scrolling text, signal
 * bars), big ⏮ ▶ ⏭ and volume, the stations as preset tiles, the multiplex on one bar.
 * Landscape: display left, stations right. Portrait (phone upright in a car mount): display on
 * top, stations below.
 */
@Composable
fun CarRadioScreen(
    d: DigitalState,
    slide: ImageBitmap?,
    frequency: Double,
    playing: Boolean,
    playEnabled: Boolean,
    found: Map<String, String>,
    scan: Pair<Int, Int>?,
    af: Boolean,
    source: String,
    usb: Boolean,
    space: AppSpace,
    cornerRadius: Dp,
    catalog: List<app.littlesdr.ui.RadioViewModel.CatalogStation>,
    onCatalog: (app.littlesdr.ui.RadioViewModel.CatalogStation) -> Unit,
    afSearching: Boolean,
    onPlay: () -> Unit,
    onStep: (Int) -> Unit,
    onService: (DabService) -> Unit,
    onTune: (Double) -> Unit,
    onScan: () -> Unit,
    onAf: (Boolean) -> Unit,
    onSource: () -> Unit,
    onSettings: () -> Unit,
    onSpace: (AppSpace) -> Unit,
    modifier: Modifier = Modifier,
) {
    var grid by rememberSaveable { mutableStateOf(false) }
    BoxWithConstraints(modifier) {
        val portrait = maxHeight > maxWidth
        val display = @Composable { m: Modifier ->
            CarDisplay(
                d, slide, frequency, playing, playEnabled, af, afSearching, source, usb, space, cornerRadius,
                onPlay, onStep, onAf, onSource, onSettings, onSpace, m,
            )
        }
        val stations = @Composable { m: Modifier ->
            Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, shape = RoundedCornerShape(cornerRadius), modifier = m) {
                Column(Modifier.fillMaxSize().padding(12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    MultiplexBar(frequency, d, found, scan, onTune, onScan, onOpenGrid = { grid = true })
                    scan?.let { (i, n) ->
                        Text("Recherche : canal ${DAB_CHANNELS[i].name} (${i + 1}/$n)", style = MaterialTheme.typography.bodyMedium)
                        LinearProgressIndicator(progress = { (i + 1f) / n }, modifier = Modifier.fillMaxWidth())
                    }
                    var query by rememberSaveable { mutableStateOf("") }
                    StationSearch(query, onQuery = { query = it })
                    if (query.isBlank()) {
                        val here = dabChannelAt(frequency)?.name
                        val known = if (playing && d.services.isNotEmpty()) emptyList() else catalog.filter { it.channel == here }
                        if (known.isNotEmpty()) KnownStations(known, onCatalog, Modifier.weight(1f).fillMaxWidth())
                        else PresetGrid(d, playing, onService, Modifier.weight(1f).fillMaxWidth())
                    }
                    else SearchResults(query, frequency, d, catalog, found, onService, onCatalog = { onCatalog(it); query = "" }, Modifier.weight(1f).fillMaxWidth())
                }
            }
        }
        if (portrait) {
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                display(Modifier.fillMaxWidth().weight(0.46f))
                stations(Modifier.fillMaxWidth().weight(0.54f))
            }
        } else {
            Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                display(Modifier.weight(1.15f).fillMaxHeight())
                stations(Modifier.weight(1f).fillMaxHeight())
            }
        }
    }
    if (grid) {
        ChannelGridSheet(frequency, found, onPick = { onTune(it.hz); grid = false }, onDismiss = { grid = false })
    }
}

/** The head unit: top bar, the big display, transport and volume */
@Composable
private fun CarDisplay(
    d: DigitalState,
    slide: ImageBitmap?,
    frequency: Double,
    playing: Boolean,
    playEnabled: Boolean,
    af: Boolean,
    afSearching: Boolean,
    source: String,
    usb: Boolean,
    space: AppSpace,
    cornerRadius: Dp,
    onPlay: () -> Unit,
    onStep: (Int) -> Unit,
    onAf: (Boolean) -> Unit,
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
            // Top bar: receiver, AF, signal, settings, space
            // The buttons on the right keep their size; on a narrow screen or with large text,
            // the receiver's name gives way (ellipsis) instead of crushing the space switcher
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Surface(onClick = onSource, shape = RoundedCornerShape(50), color = cs.surfaceContainerHigh, modifier = Modifier.weight(1f, fill = false)) {
                    Row(Modifier.padding(horizontal = 10.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(if (usb) Icons.Default.Usb else Icons.Default.Router, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(source, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                Surface(
                    onClick = { onAf(!af) },
                    shape = RoundedCornerShape(50),
                    color = if (af) cs.primary else cs.surfaceContainerHigh,
                    contentColor = if (af) cs.onPrimary else cs.onSurfaceVariant,
                ) {
                    Text("AF", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.Bold, modifier = Modifier.padding(horizontal = 11.dp, vertical = 8.dp))
                }
                Spacer(Modifier.weight(0.001f))
                SignalBars(playing, d, Modifier.clip(RoundedCornerShape(8.dp)).padding(4.dp), onClick = { status = true })
                IconButton(onClick = onSettings) { Icon(Icons.Default.Settings, "Réglages") }
                SpaceSwitcher(space = space, onSpace = onSpace, shape = CircleShape, modifier = Modifier.size(48.dp))
            }
            // The display
            Row(Modifier.weight(1f).fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(18.dp)) {
                CarArtwork(slide, d.playing, Modifier.fillMaxHeight().aspectRatio(1f, matchHeightConstraintsFirst = true))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    val ch = dabChannelAt(frequency)
                    val where = listOfNotNull(ch?.name ?: String.format(Locale.FRANCE, "%.3f MHz", frequency / 1e6), d.ensemble.ifBlank { null }).joinToString(" · ")
                    Text(
                        if (afSearching) "AF : recherche de la station… ($where)" else where,
                        style = MaterialTheme.typography.titleMedium,
                        color = if (afSearching) cs.primary else cs.onSurfaceVariant,
                        maxLines = 1,
                        modifier = Modifier.basicMarquee(),
                    )
                    val station = d.playing.takeIf { playing }
                    if (station != null) {
                        Text(station, style = MaterialTheme.typography.displaySmall, fontWeight = FontWeight.Bold, maxLines = 1, modifier = Modifier.basicMarquee())
                    } else {
                        Text(
                            when {
                                !playing -> "Radio à l'arrêt"
                                d.services.isEmpty() -> "Chargement des stations…"
                                else -> "Choisis une station"
                            },
                            // A notch under the station's name: long words don't break mid-word
                            style = MaterialTheme.typography.headlineSmall,
                            fontWeight = FontWeight.Bold,
                            maxLines = 3,
                        )
                    }
                    if (playing && d.text.isNotBlank()) {
                        Text(
                            d.text,
                            style = MaterialTheme.typography.titleLarge,
                            color = cs.primary,
                            maxLines = 1,
                            modifier = Modifier.basicMarquee(),
                        )
                    }
                }
            }
            // Transport: big targets for a hand on the move
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterHorizontally)) {
                FilledTonalIconButton(onClick = { onStep(-1) }, enabled = playing, modifier = Modifier.size(64.dp)) {
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
                FilledTonalIconButton(onClick = { onStep(1) }, enabled = playing, modifier = Modifier.size(64.dp)) {
                    Icon(Icons.Default.SkipNext, "Station suivante", Modifier.size(36.dp))
                }
            }
        }
    }
    if (status) {
        AlertDialog(
            onDismissRequest = { status = false },
            confirmButton = { TextButton(onClick = { status = false }) { Text("OK") } },
            title = { Text("Réception") },
            text = { Text(d.status.ifBlank { "Pas de réception en cours" }) },
        )
    }
}

/** 0-4 bars from the SNR (DAB) or the share of frames decoded (DRM), as on a car radio; tap for the details */
@Composable
internal fun SignalBars(playing: Boolean, d: DigitalState, modifier: Modifier, onClick: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    val bars = when {
        !playing || !d.synced -> 0
        d.kind == "drm" -> when {
            d.quality < 0.3f -> 1
            d.quality < 0.6f -> 2
            d.quality < 0.85f -> 3
            else -> 4
        }
        d.snr < 6 -> 1
        d.snr < 10 -> 2
        d.snr < 15 -> 3
        else -> 4
    }
    Surface(onClick = onClick, color = cs.surfaceContainerHigh, shape = RoundedCornerShape(10.dp), modifier = modifier) {
        Row(Modifier.padding(horizontal = 10.dp, vertical = 8.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            for (i in 1..4) {
                Box(
                    Modifier.width(5.dp).height((5 + i * 4).dp).clip(RoundedCornerShape(2.dp))
                        .background(if (i <= bars) cs.primary else cs.outlineVariant),
                )
            }
        }
    }
}

/** Slideshow, else the station's logo, else a radio */
@Composable
private fun CarArtwork(slide: ImageBitmap?, station: String?, modifier: Modifier) {
    val cs = MaterialTheme.colorScheme
    val logo = stationLogo(station)
    Box(modifier.clip(RoundedCornerShape(18.dp)).background(cs.surfaceContainerHighest), contentAlignment = Alignment.Center) {
        Crossfade(slide ?: logo, label = "art") { img ->
            if (img != null) {
                // The slideshow fills the square (zoomed, no margin); a logo keeps a margin
                if (img === slide) Image(img, "Diaporama de la station", contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                else LogoFill(img, description = "Logo de la station")
            } else {
                Icon(Icons.Rounded.Radio, null, Modifier.fillMaxSize(0.45f), tint = cs.primary.copy(alpha = 0.7f))
            }
        }
    }
}

/** `◀  11D · Métropolitain 1  ▶` + search + all channels */
@Composable
private fun MultiplexBar(
    frequency: Double,
    d: DigitalState,
    found: Map<String, String>,
    scan: Pair<Int, Int>?,
    onTune: (Double) -> Unit,
    onScan: () -> Unit,
    onOpenGrid: () -> Unit,
) {
    val cs = MaterialTheme.colorScheme
    val ch = dabChannelAt(frequency)
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        FilledTonalIconButton(onClick = { onTune(stepChannel(frequency, found, -1).hz) }, modifier = Modifier.size(48.dp)) {
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowLeft, "Multiplex précédent", Modifier.size(28.dp))
        }
        Surface(onClick = onOpenGrid, shape = RoundedCornerShape(14.dp), color = cs.surfaceContainerHigh, modifier = Modifier.weight(1f).height(48.dp)) {
            Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(ch?.name ?: "—", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = cs.primary)
                Spacer(Modifier.width(8.dp))
                Text(
                    d.ensemble.ifBlank { ch?.let { found[it.name] } ?: "" },
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
            }
        }
        FilledTonalIconButton(onClick = { onTune(stepChannel(frequency, found, 1).hz) }, modifier = Modifier.size(48.dp)) {
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, "Multiplex suivant", Modifier.size(28.dp))
        }
        FilledTonalIconButton(onClick = onScan, modifier = Modifier.size(48.dp)) {
            // Radar, not a magnifier: the magnifier is the station-name search
            Icon(if (scan != null) Icons.Default.Stop else Icons.Default.Radar, if (scan != null) "Arrêter la recherche" else "Rechercher les multiplex")
        }
    }
}

/** Radio stopped: the stations already heard on this channel; a tap starts on it */
@Composable
private fun KnownStations(
    known: List<app.littlesdr.ui.RadioViewModel.CatalogStation>,
    onCatalog: (app.littlesdr.ui.RadioViewModel.CatalogStation) -> Unit,
    modifier: Modifier,
) {
    LazyVerticalGrid(
        columns = GridCells.Adaptive(150.dp),
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(known, key = { it.channel + it.sid }) { s -> PresetTile(DabService(s.sid, s.name, true), false) { onCatalog(s) } }
    }
}

/** Search by station name, over this multiplex and every one heard before */
@Composable
private fun StationSearch(query: String, onQuery: (String) -> Unit) {
    OutlinedTextField(
        value = query,
        onValueChange = onQuery,
        placeholder = { Text("Chercher une station…", maxLines = 1) },
        leadingIcon = { Icon(Icons.Default.Search, null) },
        trailingIcon = if (query.isNotEmpty()) {
            { IconButton(onClick = { onQuery("") }) { Icon(Icons.Default.Close, "Effacer") } }
        } else null,
        singleLine = true,
        shape = RoundedCornerShape(16.dp),
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        modifier = Modifier.fillMaxWidth(),
    )
}

/** Stations whose name matches: here first, then on the other multiplexes (with their channel) */
@Composable
private fun SearchResults(
    query: String,
    frequency: Double,
    d: DigitalState,
    catalog: List<app.littlesdr.ui.RadioViewModel.CatalogStation>,
    found: Map<String, String>,
    onService: (DabService) -> Unit,
    onCatalog: (app.littlesdr.ui.RadioViewModel.CatalogStation) -> Unit,
    modifier: Modifier,
) {
    val cs = MaterialTheme.colorScheme
    val q = query.trim().lowercase(Locale.ROOT)
    val here = dabChannelAt(frequency)?.name
    val local = d.services.filter { q in it.label.lowercase(Locale.ROOT) }.sortedBy { it.label.lowercase() }
    val elsewhere = catalog.filter { q in it.name.lowercase(Locale.ROOT) && it.channel != here && local.none { l -> l.sid == it.sid } }
    if (local.isEmpty() && elsewhere.isEmpty()) {
        Box(modifier, contentAlignment = Alignment.Center) {
            Text(
                if (catalog.isEmpty()) "Aucune station connue : lance une recherche des multiplex (loupe en haut)"
                else "Aucune station « $query » dans les multiplex entendus",
                style = MaterialTheme.typography.bodyLarge,
                color = cs.onSurfaceVariant,
            )
        }
        return
    }
    LazyVerticalGrid(
        columns = GridCells.Adaptive(150.dp),
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(local, key = { "h" + it.sid }) { s -> PresetTile(s, s.label == d.playing) { onService(s) } }
        items(elsewhere, key = { "o" + it.channel + it.sid }) { s ->
            PresetTile(DabService(s.sid, s.name, true), false, where = "${s.channel} · ${found[s.channel] ?: ""}") { onCatalog(s) }
        }
    }
}

/** The stations as big preset tiles with their logos */
@Composable
private fun PresetGrid(d: DigitalState, playing: Boolean, onService: (DabService) -> Unit, modifier: Modifier) {
    val cs = MaterialTheme.colorScheme
    val services = d.services.sortedWith(compareBy({ !it.dabPlus }, { it.label.lowercase() }))
    if (services.isEmpty()) {
        Box(modifier, contentAlignment = Alignment.Center) {
            Text(
                when {
                    !playing -> "Appuie sur ▶ : les stations de ce multiplex s'afficheront ici"
                    d.synced -> "Lecture de la liste des stations…"
                    else -> "Pas de multiplex ici : change de canal ou lance une recherche"
                },
                style = MaterialTheme.typography.bodyLarge,
                color = cs.onSurfaceVariant,
            )
        }
        return
    }
    LazyVerticalGrid(
        columns = GridCells.Adaptive(150.dp),
        modifier = modifier,
        contentPadding = PaddingValues(bottom = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(services, key = { it.sid }) { s -> PresetTile(s, s.label == d.playing) { onService(s) } }
    }
}

@Composable
private fun PresetTile(s: DabService, current: Boolean, where: String? = null, onClick: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        enabled = s.dabPlus,
        shape = RoundedCornerShape(16.dp),
        color = if (current) cs.primaryContainer else cs.surfaceContainerHigh,
        border = if (current) BorderStroke(2.dp, cs.primary) else null,
        modifier = Modifier.fillMaxWidth().height(84.dp),
    ) {
        Row(Modifier.padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            val logo = stationLogo(s.label)
            Box(Modifier.size(46.dp).clip(RoundedCornerShape(10.dp)).background(cs.surfaceContainerHighest), contentAlignment = Alignment.Center) {
                if (logo != null) LogoFill(logo)
                else Text(s.label.take(2).uppercase(Locale.ROOT), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, color = cs.primary)
            }
            Spacer(Modifier.width(8.dp))
            Column(Modifier.weight(1f)) {
                WordFitText(
                    s.label,
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = if (current) FontWeight.Bold else FontWeight.Medium),
                    color = if (!s.dabPlus) cs.onSurfaceVariant else if (current) cs.onPrimaryContainer else cs.onSurface,
                    maxLines = 2,
                )
                if (!s.dabPlus) Text("MP2 : non pris en charge", style = MaterialTheme.typography.labelSmall, color = cs.onSurfaceVariant)
                where?.let { Text(it, style = MaterialTheme.typography.labelMedium, color = cs.primary, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
        }
    }
}

/**
 * The slideshow picture, blurred, behind a whole card: some of its colour around the player.
 * A veil of the card's colour keeps the text readable (thicker before Android 12, which
 * cannot blur).
 */
@Composable
fun SlideBackdrop(slide: ImageBitmap?) {
    val cs = MaterialTheme.colorScheme
    Crossfade(slide, label = "backdrop", modifier = Modifier.fillMaxSize()) { img ->
        if (img != null) Box(Modifier.fillMaxSize()) {
            Image(img, null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize().blur(48.dp))
            val veil = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) 0.62f else 0.85f
            Box(Modifier.fillMaxSize().background(cs.surfaceContainerLow.copy(alpha = veil)))
        }
    }
}
