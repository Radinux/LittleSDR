package app.littlesdr.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.RecorderState
import app.littlesdr.ui.formatHz
import app.littlesdr.ui.theme.ReadoutStyle
import java.io.File
import java.util.Locale

/**
 * Recording to WAV: the demodulated sound, or the baseband (IQ) as a 500 kHz slice around the
 * tuned frequency, which can be replayed / decoded later by other SDR software.
 */
@Composable
fun RecordTab(
    rec: RecorderState,
    recordings: List<File>,
    playing: Boolean,
    folder: String,
    onWatch: (Boolean) -> Unit,
    onSet: (String, Double) -> Unit,
    onDelete: (File) -> Unit,
) {
    DisposableEffect(Unit) {
        onWatch(true)
        onDispose { onWatch(false) }
    }
    var deleting by remember { mutableStateOf<File?>(null) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SectionLabel("Enregistrement")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(selected = rec.audio, enabled = !rec.recording, onClick = { onSet("mode", 1.0) }, label = { Text("Son") })
            FilterChip(selected = !rec.audio, enabled = !rec.recording, onClick = { onSet("mode", 0.0) }, label = { Text("Baseband (IQ)") })
        }
        if (rec.audio) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Fichier", style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                FilterChip(selected = rec.stereo, enabled = !rec.recording, onClick = { onSet("stereo", 1.0) }, label = { Text("Stéréo") })
                FilterChip(selected = !rec.stereo, enabled = !rec.recording, onClick = { onSet("stereo", 0.0) }, label = { Text("Mono") })
            }
        } else {
            val rate = rec.bbRate
            Text(
                if (rate > 0) "${formatHz(rate)} autour de ${formatHz(rec.bbCenter)} · ≈ ${formatBytes((rate * 4).toLong())}/s"
                else "Lance la réception pour voir la portion enregistrée",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // Record / stop, with time, size and level while recording
        Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (rec.recording) {
                        Box(Modifier.size(10.dp).clip(CircleShape).background(MaterialTheme.colorScheme.error))
                        Spacer(Modifier.width(8.dp))
                    }
                    Text(clock(rec.seconds), style = ReadoutStyle, modifier = Modifier.weight(1f))
                    if (rec.recording) {
                        Text(formatBytes(rec.bytes.toLong()), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (rec.audio && rec.level.size == 2) {
                    LevelBar(rec.level[0])
                    LevelBar(rec.level[1])
                }
                Button(
                    onClick = { onSet(if (rec.recording) "stop" else "start", 0.0) },
                    colors = if (rec.recording) ButtonDefaults.buttonColors() else
                        ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error, contentColor = MaterialTheme.colorScheme.onError),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(if (rec.recording) Icons.Default.Stop else Icons.Default.FiberManualRecord, null, Modifier.size(18.dp))
                    Text(if (rec.recording) "Arrêter" else "Enregistrer", Modifier.padding(start = 8.dp))
                }
                if (rec.recording && rec.file.isNotEmpty()) {
                    Text(File(rec.file).name, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
                } else if (!playing) {
                    Text("La réception est arrêtée : le fichier resterait silencieux.", style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }

        SectionLabel(if (recordings.isEmpty()) "Enregistrements" else "Enregistrements · ${recordings.size}")
        Text(folder, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        recordings.take(100).forEach { f -> RecordingRow(f, onDelete = { deleting = f }) }
    }

    deleting?.let { f ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Supprimer l'enregistrement ?") },
            text = { Text(f.name) },
            confirmButton = { TextButton(onClick = { onDelete(f); deleting = null }) { Text("Supprimer") } },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text("Annuler") } },
        )
    }
}

@Composable
private fun RecordingRow(f: File, onDelete: () -> Unit) {
    val context = LocalContext.current
    val baseband = "_baseband_" in f.name
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f)) {
            Text(
                f.nameWithoutExtension.substringBefore("_baseband_").substringBefore("_audio_").replace('_', ' '),
                style = MaterialTheme.typography.bodyMedium,
            )
            Text(
                "${if (baseband) "Baseband" else "Son"} · ${formatBytes(f.length())}",
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (!baseband) IconButton(onClick = { FileActions.open(context, f) }) { Icon(Icons.Default.PlayArrow, "Écouter") }
        IconButton(onClick = { FileActions.share(context, f) }) { Icon(Icons.Default.Share, "Partager") }
        IconButton(onClick = onDelete) { Icon(Icons.Default.Delete, "Supprimer") }
    }
}

/** Peak level, -60..0 dB. */
@Composable
private fun LevelBar(db: Float) {
    val f = ((db + 60f) / 60f).coerceIn(0f, 1f)
    Box(Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)).background(MaterialTheme.colorScheme.surfaceContainerHighest)) {
        Box(
            Modifier.fillMaxWidth(f).fillMaxHeight()
                .background(if (db > -3f) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary),
        )
    }
}

private fun clock(s: Double): String {
    val t = s.toLong()
    return if (t >= 3600) String.format(Locale.ROOT, "%d:%02d:%02d", t / 3600, t / 60 % 60, t % 60)
    else String.format(Locale.ROOT, "%02d:%02d", t / 60, t % 60)
}
