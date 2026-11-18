package app.littlesdr.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.CwLine
import app.littlesdr.engine.CwState
import app.littlesdr.engine.Mode
import app.littlesdr.ui.formatHz
import app.littlesdr.ui.theme.ReadoutStyle
import java.util.Locale

/**
 * Morse decoding (SDR++Brown's TinyCW): every keyed signal of the amateur CW segment in view,
 * plus the tuned frequency when the radio is in CW mode.
 */
@Composable
fun MorseTab(
    cw: CwState,
    mode: Mode?,
    onEnable: (Boolean) -> Unit,
    onWatch: (Boolean) -> Unit,
    onTune: (Double) -> Unit,
) {
    DisposableEffect(Unit) {
        onWatch(true)
        onDispose { onWatch(false) }
    }

    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                SectionLabel("Décodage morse")
                Text(
                    statusText(cw),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = cw.enabled, onCheckedChange = onEnable, enabled = cw.ready)
        }
        if (!cw.enabled) {
            Text(
                "Décode tous les signaux morse de la portion CW d'une bande radioamateur (40 m, 20 m…), " +
                    "et la fréquence accordée en mode CW. Consomme du processeur pendant la réception.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            return@Column
        }

        // Tuned frequency
        Surface(
            shape = RoundedCornerShape(16.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("Fréquence accordée", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
                val focus = cw.focus.text.takeLast(160)
                Text(
                    when {
                        mode != Mode.CW -> "Passe en mode CW pour décoder la fréquence accordée."
                        focus.isBlank() -> "En écoute…"
                        else -> focus
                    },
                    style = if (mode == Mode.CW && focus.isNotBlank()) MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace)
                    else MaterialTheme.typography.bodySmall,
                    color = if (mode == Mode.CW && focus.isNotBlank()) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.heightIn(min = 20.dp),
                )
            }
        }

        // Every decoded signal of the segment
        if (cw.lines.isNotEmpty()) {
            SectionLabel("Dans la bande · ${cw.lines.size}")
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                cw.lines.forEach { line -> CwRow(line, onTune) }
            }
        }
    }
}

private fun statusText(cw: CwState): String = when {
    !cw.ready -> "Décodeur indisponible"
    !cw.enabled -> "Arrêté"
    !cw.inBand -> "Pas de portion CW radioamateur dans la vue"
    else -> buildString {
        append("Bande ${cw.band.ifBlank { "?" }}")
        if (cw.hi > cw.lo) append(" · ${formatHz(cw.lo)} – ${formatHz(cw.hi)}")
        append(" · ${cw.channels} signal${if (cw.channels > 1) "s" else ""}")
    }
}

@Composable
private fun CwRow(line: CwLine, onTune: (Double) -> Unit) {
    val primary = MaterialTheme.colorScheme.primary
    // Text right-aligned on its newest characters, the latest ones highlighted
    val text = line.text.takeLast(48)
    val fresh = line.fresh.takeLast(text.length)
    val old = text.dropLast(fresh.length.coerceAtMost(text.length).takeIf { text.endsWith(fresh) } ?: 0)
    Row(
        Modifier
            .fillMaxWidth()
            .clickable { onTune(line.f) }
            .padding(vertical = 6.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            String.format(Locale.FRANCE, "%.1f", line.f / 1000.0),
            style = ReadoutStyle,
            color = primary,
            modifier = Modifier.width(72.dp),
        )
        Text(
            buildAnnotatedString {
                append(old)
                if (text.endsWith(fresh) && fresh.isNotEmpty()) withStyle(SpanStyle(color = primary)) { append(fresh) }
            },
            style = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
            maxLines = 1,
            overflow = TextOverflow.StartEllipsis,
            modifier = Modifier.weight(1f),
        )
    }
}
