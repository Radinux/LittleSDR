package app.littlesdr.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.DeleteSweep
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.engine.Ft8Bands
import app.littlesdr.engine.Ft8Decoder
import app.littlesdr.engine.Ft8Result
import app.littlesdr.engine.Maidenhead
import app.littlesdr.ui.components.Ft8Map
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import app.littlesdr.ui.theme.ReadoutStyle
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import kotlin.math.roundToInt

private val UTC_TIME = SimpleDateFormat("HH:mm:ss", Locale.ROOT).apply { timeZone = TimeZone.getTimeZone("UTC") }

@OptIn(ExperimentalMaterial3Api::class, androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
fun Ft8Screen(vm: RadioViewModel, onBack: () -> Unit) {
    val ft8 by vm.ft8.collectAsStateWithLifecycle()
    val state by vm.state.collectAsStateWithLifecycle()
    var cqOnly by rememberSaveable { mutableStateOf(false) }
    var showMap by rememberSaveable { mutableStateOf(false) }
    val traffic by vm.ft8Traffic.collectAsStateWithLifecycle()
    val advanced by vm.advanced.collectAsStateWithLifecycle()
    val home = remember(advanced.operator.location) { Maidenhead.toLatLon(advanced.operator.location) }

    DisposableEffect(Unit) {
        vm.watchFt8(true)
        onDispose { vm.watchFt8(false) }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("FT8 / FT4") },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Retour") } },
                actions = {
                    SingleChoiceSegmentedButtonRow(Modifier.padding(end = 8.dp)) {
                        SegmentedButton(selected = !showMap, onClick = { showMap = false }, shape = SegmentedButtonDefaults.itemShape(0, 2)) { Text("Liste") }
                        SegmentedButton(selected = showMap, onClick = { showMap = true }, shape = SegmentedButtonDefaults.itemShape(1, 2)) { Text("Carte") }
                    }
                    IconButton(onClick = vm::ft8Clear, enabled = ft8.results.isNotEmpty()) {
                        Icon(Icons.Default.DeleteSweep, "Effacer les décodages")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
            )
        },
        containerColor = MaterialTheme.colorScheme.surface,
    ) { padding ->
        Row(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 16.dp, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // ---- controls
            Column(
                Modifier.widthIn(max = 300.dp).weight(0.38f).fillMaxHeight().verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Card(
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                    shape = RoundedCornerShape(24.dp),
                ) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f)) {
                                Text("Décodeur", style = MaterialTheme.typography.titleMedium)
                                Text(
                                    when {
                                        !ft8.enabled -> "Arrêté"
                                        !state.playing -> "En attente : lance la réception"
                                        else -> "Actif"
                                    },
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Switch(checked = ft8.enabled, onCheckedChange = { vm.ft8Set("enabled", it) })
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            FilterChip(selected = ft8.ft8.processing, onClick = { vm.ft8Set("ft8", !ft8.ft8.processing) }, label = { Text("FT8") })
                            FilterChip(selected = ft8.ft4.processing, onClick = { vm.ft8Set("ft4", !ft8.ft4.processing) }, label = { Text("FT4") })
                            Spacer(Modifier.weight(1f))
                            FilterChip(selected = cqOnly, onClick = { cqOnly = !cqOnly }, label = { Text("CQ seuls") })
                        }
                        if (ft8.enabled) {
                            HorizontalDivider()
                            DecoderStatus("FT8", ft8.ft8)
                            DecoderStatus("FT4", ft8.ft4)
                        }
                    }
                }

                Text("Bande", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                // Highlight from the VFO (updated instantly on tap); the decoder's own report lags
                // behind fast band changes and is only a fallback when the VFO is off-channel.
                val vfo = state.vfoFrequency
                val vfoBand = Ft8Bands.FT8.firstOrNull { (_, hz) -> vfo >= hz && vfo <= hz + 3_000 }?.second
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Ft8Bands.FT8.forEach { (band, hz) ->
                        FilterChip(
                            selected = if (vfoBand != null) vfoBand == hz else ft8.ft8.dial == hz,
                            onClick = { vm.tuneDigital(hz) },
                            label = { Text(band) },
                        )
                    }
                }
                Text(
                    "Le décodeur écoute la fréquence FT8/FT4 standard comprise dans la bande affichée. " +
                        "En HF, un RTL-SDR a besoin de l'échantillonnage direct ou d'un convertisseur (Réglages › Entrée). " +
                        "L'heure du téléphone doit être exacte à la seconde près.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
            }

            // ---- decodes
            val shown = remember(ft8.results, cqOnly) { if (cqOnly) ft8.results.filter { it.isCq } else ft8.results }
            Card(
                Modifier.weight(0.62f).fillMaxHeight(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                shape = RoundedCornerShape(24.dp),
            ) {
                if (showMap) {
                    Box(Modifier.fillMaxSize()) {
                        Ft8Map(traffic, home, Modifier.fillMaxSize())
                        Text(
                            "${traffic.stations.count { it.heard }} stations · ${traffic.links.size} liaisons (2 min)",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.align(Alignment.BottomStart).padding(14.dp),
                        )
                        if (home == null) {
                            Text(
                                "Ajoute ton locator dans Réglages › Opérateur pour te voir sur la carte",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.align(Alignment.TopCenter).padding(10.dp),
                            )
                        }
                    }
                } else if (shown.isEmpty()) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(
                            if (ft8.enabled) "Aucun décodage pour l'instant" else "Active le décodeur pour commencer",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                } else {
                    val slots = remember(shown) { shown.groupBy { it.slotStart } }
                    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 12.dp)) {
                        slots.forEach { (slot, decodes) ->
                            stickyHeader(key = "slot-$slot") { SlotHeader(slot, decodes.size) }
                            items(decodes, key = { "${it.mode}|${it.call}|${it.message}|$slot" }) { r -> DecodeItem(r) }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun DecoderStatus(name: String, d: Ft8Decoder) {
    if (!d.processing) return
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(name, style = MaterialTheme.typography.labelLarge, modifier = Modifier.width(40.dp))
        Text(
            when {
                d.error.isNotBlank() -> d.error
                d.dial == 0L -> "Aucune fréquence $name dans la bande affichée"
                else -> "${formatFrequency(d.dial.toDouble())} MHz · ${d.lastDecodeCount} au dernier cycle"
            },
            style = MaterialTheme.typography.bodySmall,
            color = if (d.error.isNotBlank()) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(1f),
        )
        if (d.decoding) CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
    }
}

@Composable
private fun SlotHeader(slot: Long, count: Int) {
    Row(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(start = 16.dp, end = 16.dp, top = 10.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(UTC_TIME.format(Date(slot)) + " UTC", style = ReadoutStyle, color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.width(8.dp))
        Text(
            "· $count décodage${if (count > 1) "s" else ""}",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** One decode: SNR badge, message (CQ highlighted), then mode · Δf · country · distance. */
@Composable
private fun DecodeItem(r: Ft8Result) {
    val cq = r.isCq
    val cs = MaterialTheme.colorScheme
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 10.dp, vertical = 3.dp)
            .clip(RoundedCornerShape(14.dp))
            .background(if (cq) cs.primaryContainer.copy(alpha = 0.55f) else cs.surfaceContainer)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SnrBadge(r.snr)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(
                r.message,
                fontFamily = FontFamily.Monospace,
                fontWeight = if (cq) FontWeight.SemiBold else FontWeight.Normal,
                color = if (cq) cs.onPrimaryContainer else cs.onSurface,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                buildString {
                    append(r.mode)
                    append(" · ${r.audioHz} Hz")
                    if (r.country.isNotBlank()) append(" · ${r.country}")
                    if (r.distanceKm > 0) append(" · ${r.distanceKm.roundToInt()} km")
                },
                style = MaterialTheme.typography.bodySmall,
                color = cs.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** SNR in dB, colored from weak (muted) to strong (primary). */
@Composable
private fun SnrBadge(snr: Double) {
    val cs = MaterialTheme.colorScheme
    val strong = snr >= -5
    val weak = snr < -18
    Box(
        Modifier
            .clip(RoundedCornerShape(10.dp))
            .background(
                when {
                    strong -> cs.primary
                    weak -> cs.surfaceContainerHighest
                    else -> cs.secondaryContainer
                }
            )
            .padding(horizontal = 8.dp, vertical = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            "${if (snr >= 0) "+" else ""}${snr.roundToInt()}",
            style = ReadoutStyle,
            color = when {
                strong -> cs.onPrimary
                weak -> cs.onSurfaceVariant
                else -> cs.onSecondaryContainer
            },
        )
    }
}
