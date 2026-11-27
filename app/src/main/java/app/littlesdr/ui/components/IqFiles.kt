package app.littlesdr.ui.components

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AudioFile
import androidx.compose.material.icons.filled.FileOpen
import app.littlesdr.ui.theme.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.io.File
import java.util.Locale

/**
 * IQ recordings to play: a file from anywhere on the phone (system picker; copied into the
 * app's IQ folder under its own name, which tells the frequency), or one already here
 * (imported before, or recorded by the app). Tap one to play it.
 */
@Composable
fun IqFilePicker(
    files: List<File>,
    current: String,
    importing: String?,
    enabled: Boolean,
    onRefresh: () -> Unit,
    onImport: (android.net.Uri) -> Unit,
    onPick: (File) -> Unit,
) {
    val cs = MaterialTheme.colorScheme
    LaunchedEffect(Unit) { onRefresh() }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let(onImport) }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        FilledTonalButton(
            onClick = { launcher.launch(arrayOf("audio/*", "application/octet-stream", "*/*")) },
            enabled = enabled && importing == null,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Default.FileOpen, null, Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text("Importer un fichier IQ…")
        }
        importing?.let {
            Text("Copie de « $it »…", style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            LinearProgressIndicator(Modifier.fillMaxWidth())
        }
        if (files.isEmpty() && importing == null) {
            Text(
                "Fichiers WAV IQ (SDR++, SDR#, HDSDR ou les enregistrements « bande de base » de l'app). La fréquence est lue dans le nom du fichier.",
                style = MaterialTheme.typography.bodySmall,
                color = cs.onSurfaceVariant,
            )
        }
        files.forEach { f ->
            val sel = f.path == current
            Surface(
                onClick = { onPick(f) },
                enabled = enabled,
                shape = RoundedCornerShape(12.dp),
                color = if (sel) cs.secondaryContainer else cs.surfaceContainer,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(Modifier.padding(horizontal = 12.dp, vertical = 9.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Default.AudioFile, null, Modifier.size(20.dp), tint = if (sel) cs.onSecondaryContainer else cs.primary)
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text(f.name, style = MaterialTheme.typography.bodyMedium, fontWeight = if (sel) FontWeight.SemiBold else null, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(
                            listOfNotNull(
                                Regex("(\\d+)Hz").find(f.name)?.groupValues?.get(1)?.toDoubleOrNull()?.let { hz ->
                                    if (hz >= 1e6) String.format(Locale.FRANCE, "%.3f MHz", hz / 1e6) else String.format(Locale.FRANCE, "%.1f kHz", hz / 1e3)
                                },
                                String.format(Locale.FRANCE, "%.0f Mo", f.length() / 1e6),
                                if (f.parentFile?.name == "Baseband") "enregistrement" else null,
                            ).joinToString(" · "),
                            style = MaterialTheme.typography.labelSmall,
                            color = cs.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}
