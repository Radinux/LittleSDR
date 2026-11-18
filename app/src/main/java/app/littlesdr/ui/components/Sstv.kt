package app.littlesdr.ui.components

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.OpenInNew
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.SstvState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

/**
 * SSTV images (Martin, Scottie, Robot, PD), decoded from the demodulated audio with automatic
 * mode detection (VIS). Finished images are saved and listed below.
 */
@Composable
fun SstvTab(
    sstv: SstvState,
    image: Bitmap?,
    gallery: List<File>,
    onEnable: (Boolean) -> Unit,
    onStartManual: (Int) -> Unit,
    onFinish: () -> Unit,
    onRefreshGallery: () -> Unit,
    onDelete: (File) -> Unit,
) {
    LaunchedEffect(Unit) { onRefreshGallery() }
    var viewing by remember { mutableStateOf<File?>(null) }

    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                SectionLabel("Images SSTV")
                Text(
                    statusText(sstv),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = sstv.enabled, onCheckedChange = onEnable)
        }

        if (!sstv.enabled) {
            Text(
                "Décode les images SSTV dans le son de la radio : ISS sur 145,800 MHz en NFM, " +
                    "14,230 MHz en USB… Modes Martin, Scottie, Robot et PD, reconnus automatiquement.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            // Image being received (or the last one)
            Box(
                Modifier
                    .fillMaxWidth()
                    .aspectRatio(if (sstv.width > 0 && sstv.height > 0) sstv.width.toFloat() / sstv.height else 4f / 3f)
                    .clip(RoundedCornerShape(12.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHighest),
                contentAlignment = Alignment.Center,
            ) {
                if (image != null) {
                    Image(
                        image.asImageBitmap(), contentDescription = "Image SSTV",
                        contentScale = ContentScale.Fit, filterQuality = FilterQuality.Medium,
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    Text("En attente d'une image…", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (sstv.receiving && sstv.height > 0) {
                LinearProgressIndicator(progress = { sstv.line / sstv.height.toFloat() }, modifier = Modifier.fillMaxWidth())
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                if (sstv.receiving) {
                    OutlinedButton(onClick = onFinish) { Text("Terminer l'image") }
                }
                ManualStart(sstv.modes, onStartManual)
            }
        }

        if (gallery.isNotEmpty()) {
            SectionLabel("Images reçues · ${gallery.size}")
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                gallery.take(60).forEach { f -> Thumbnail(f) { viewing = f } }
            }
        }
    }

    viewing?.let { f ->
        ImageDialog(f, onDismiss = { viewing = null }, onDelete = { onDelete(f); viewing = null })
    }
}

private fun statusText(s: SstvState): String = when {
    !s.enabled -> "Arrêté"
    s.receiving -> "Réception ${s.mode} · ligne ${s.line}/${s.height}"
    else -> "En attente d'un signal SSTV…"
}

/** For a transmission whose VIS header was missed: start decoding now in a chosen mode. */
@Composable
private fun ManualStart(modes: List<String>, onStart: (Int) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }, enabled = modes.isNotEmpty()) { Text("Démarrer en mode…") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            modes.forEachIndexed { i, m ->
                DropdownMenuItem(text = { Text(m) }, onClick = { open = false; onStart(i) })
            }
        }
    }
}

@Composable
private fun Thumbnail(f: File, onClick: () -> Unit) {
    val bmp by produceState<Bitmap?>(null, f) {
        value = withContext(Dispatchers.IO) {
            runCatching { BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = 4 }) }.getOrNull()
        }
    }
    Box(
        Modifier
            .size(width = 96.dp, height = 72.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .clickable(onClick = onClick),
    ) {
        bmp?.let { Image(it.asImageBitmap(), f.name, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxWidth().height(72.dp)) }
    }
}

@Composable
private fun ImageDialog(f: File, onDismiss: () -> Unit, onDelete: () -> Unit) {
    val context = LocalContext.current
    val bmp by produceState<Bitmap?>(null, f) {
        value = withContext(Dispatchers.IO) { runCatching { BitmapFactory.decodeFile(f.path) }.getOrNull() }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(f.nameWithoutExtension.replace('_', ' '), maxLines = 1, overflow = TextOverflow.Ellipsis) },
        text = {
            bmp?.let {
                Image(it.asImageBitmap(), f.name, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)))
            }
        },
        confirmButton = {
            Row {
                IconButton(onClick = { FileActions.share(context, f) }) { Icon(Icons.Default.Share, "Partager") }
                IconButton(onClick = { FileActions.open(context, f) }) { Icon(Icons.Default.OpenInNew, "Ouvrir") }
                IconButton(onClick = onDelete) { Icon(Icons.Default.Delete, "Supprimer") }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Fermer") } },
        modifier = Modifier.width(560.dp),
    )
}
