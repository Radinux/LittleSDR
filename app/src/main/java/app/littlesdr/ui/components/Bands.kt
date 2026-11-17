package app.littlesdr.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.BandPreset
import app.littlesdr.engine.BandPresets
import app.littlesdr.engine.PresetEntry
import app.littlesdr.ui.formatFrequency
import app.littlesdr.ui.theme.ReadoutStyle
import kotlin.math.abs

/** Band presets: pick a band (tunes its default entry), then a sub-band or a numbered channel. */
@Composable
fun BandsTab(vfoFrequency: Double, onApply: (PresetEntry) -> Unit) {
    // Band currently shown: the last one picked, or the one containing the VFO
    val containing = BandPresets.ALL.firstOrNull { b -> b.entries.any { abs(it.frequency - vfoFrequency) < 1.0 } }
    var shown by rememberSaveable { mutableStateOf(containing?.name ?: "FM") }
    val band = BandPresets.ALL.first { it.name == shown }

    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            BandPresets.ALL.forEach { b ->
                FilterChip(
                    selected = b.name == shown,
                    onClick = {
                        shown = b.name
                        onApply(b.entries[b.defaultIndex])
                    },
                    label = { Text(b.name) },
                )
            }
        }
        Text(band.description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (band.entries.size > 1) {
            if (band.channels) ChannelGrid(band, vfoFrequency, onApply) else SubBands(band, vfoFrequency, onApply)
        }
    }
}

@Composable
private fun SubBands(band: BandPreset, vfo: Double, onApply: (PresetEntry) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        band.entries.forEach { e ->
            FilterChip(
                selected = abs(e.frequency - vfo) < 1.0,
                onClick = { onApply(e) },
                label = { Text("${e.label} · ${formatFrequency(e.frequency)}") },
            )
        }
    }
}

/** Numbered channels as small rounded tiles (CB 40, PMR 16). */
@Composable
private fun ChannelGrid(band: BandPreset, vfo: Double, onApply: (PresetEntry) -> Unit) {
    val current = band.entries.firstOrNull { abs(it.frequency - vfo) < 1.0 }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        FlowRow(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            band.entries.forEach { e ->
                val sel = e == current
                Box(
                    Modifier
                        .size(44.dp)
                        .clip(RoundedCornerShape(14.dp))
                        .background(if (sel) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainer)
                        .clickable { onApply(e) },
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        e.label,
                        style = MaterialTheme.typography.labelLarge,
                        color = if (sel) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                    )
                }
            }
        }
        current?.let {
            Text(
                "Canal ${it.label} · ${formatFrequency(it.frequency)} MHz",
                style = ReadoutStyle,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(top = 2.dp),
            )
        }
    }
}
