package app.littlesdr.ui.components

import app.littlesdr.ui.theme.pillBorder
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxHeight
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Flight
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.SatelliteAlt
import androidx.compose.material.icons.filled.Save
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import app.littlesdr.ui.theme.FilledTonalButton
import app.littlesdr.ui.theme.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.LRPT_SATELLITES
import app.littlesdr.engine.LrptState
import app.littlesdr.engine.Mode
import java.util.Locale

/**
 * The faces of the app, each with its own screen and receiver. Stored by ordinal: new
 * spaces go at the end ([menuOrder] decides how they are listed).
 */
enum class AppSpace(val label: String, val icon: ImageVector) {
    SDR("SDR", Icons.Default.GraphicEq),
    DAB("DAB+", Icons.Default.Radio),
    LRPT("LRPT", Icons.Default.SatelliteAlt),
    DRM("DRM", Icons.Default.Public),
    ADSB("ADS-B", Icons.Default.Flight);

    fun accepts(m: Mode): Boolean = when (this) {
        SDR -> !m.isDigital
        DAB -> m == Mode.DAB
        DRM -> m == Mode.DRM
        LRPT -> m == Mode.LRPT
        ADSB -> m == Mode.ADSB
    }

    companion object {
        val menuOrder = listOf(SDR, DAB, DRM, LRPT, ADSB)
    }
}

/**
 * Small cap at the top right of the control panel: shows the current space, tapping it
 * drops the list of spaces down. Switching changes the interface and the receiver.
 */
@Composable
fun SpaceSwitcher(space: AppSpace, onSpace: (AppSpace) -> Unit, shape: Shape, modifier: Modifier = Modifier) {
    var open by remember { mutableStateOf(false) }
    val cs = MaterialTheme.colorScheme
    Box(modifier) {
        Surface(
            onClick = { open = true },
            shape = shape,
            color = if (open) cs.secondaryContainer else cs.surfaceContainerLow,
            border = pillBorder(),
            modifier = Modifier.fillMaxSize().semantics { contentDescription = "Changer d'interface (${space.label})" },
        ) {
            Row(horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
                Icon(space.icon, null, tint = cs.primary)
                Icon(Icons.Default.ArrowDropDown, null, Modifier.size(18.dp), tint = cs.onSurfaceVariant)
            }
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            AppSpace.menuOrder.forEach { s ->
                val current = s == space
                DropdownMenuItem(
                    text = { Text(s.label, fontWeight = if (current) FontWeight.Bold else FontWeight.Normal) },
                    onClick = { open = false; if (!current) onSpace(s) },
                    leadingIcon = { Icon(s.icon, null) },
                    trailingIcon = if (current) { { Icon(Icons.Default.Check, "Interface actuelle") } } else null,
                    colors = if (current) MenuDefaults.itemColors(
                        textColor = cs.primary, leadingIconColor = cs.primary, trailingIconColor = cs.primary,
                    ) else MenuDefaults.itemColors(),
                )
            }
        }
    }
}

/**
 * A control panel cut like the SDR one: a 64 dp top bar split in two ([head] | the space
 * switcher, 72 dp, at the top right corner), the panel's body below. Every space keeps the
 * switcher at the same place.
 */
@Composable
fun SpaceCappedPanel(
    space: AppSpace,
    onSpace: (AppSpace) -> Unit,
    cornerRadius: Dp,
    modifier: Modifier = Modifier,
    head: @Composable (Modifier) -> Unit = { SpaceTitle(space, it) },
    content: @Composable ColumnScope.() -> Unit,
) {
    val r = cornerRadius
    val cs = MaterialTheme.colorScheme
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth().height(64.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Surface(
                color = cs.surfaceContainerLow,
                border = pillBorder(),
                shape = RoundedCornerShape(topStart = r, topEnd = 6.dp, bottomStart = 6.dp, bottomEnd = 6.dp),
                modifier = Modifier.weight(1f).fillMaxHeight(),
            ) { head(Modifier.fillMaxSize()) }
            SpaceSwitcher(
                space = space,
                onSpace = onSpace,
                shape = RoundedCornerShape(topStart = 6.dp, topEnd = r, bottomStart = 6.dp, bottomEnd = 6.dp),
                modifier = Modifier.width(72.dp).fillMaxHeight(),
            )
        }
        Surface(
            color = cs.surfaceContainerLow,
            border = pillBorder(),
            shape = RoundedCornerShape(topStart = 6.dp, topEnd = 6.dp, bottomStart = r, bottomEnd = r),
            modifier = Modifier.fillMaxWidth().weight(1f),
        ) { Column(Modifier.fillMaxSize(), content = content) }
    }
}

/** Top cap of the right panel outside the SDR space: the space's name */
@Composable
fun SpaceTitle(space: AppSpace, modifier: Modifier = Modifier) {
    Row(modifier.padding(horizontal = 18.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(space.icon, null, tint = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.width(10.dp))
        Text(space.label, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

// ---------------------------------------------------------------------------------- LRPT

/** Left side in the LRPT space: the picture as it comes down */
@Composable
fun LrptImagePanel(image: ImageBitmap?, l: LrptState, playing: Boolean, modifier: Modifier = Modifier) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLowest, shape = RoundedCornerShape(20.dp), border = pillBorder(), modifier = modifier) {
        if (image != null) {
            Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState(), reverseScrolling = true), contentAlignment = Alignment.TopCenter) {
                Image(image, "Image Meteor", contentScale = ContentScale.FillWidth, modifier = Modifier.fillMaxWidth())
            }
        } else {
            Column(Modifier.fillMaxSize().padding(24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(Icons.Default.SatelliteAlt, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(12.dp))
                Text(
                    when {
                        !playing -> "Appuie sur ▶ au début d'un passage"
                        l.locked -> "Signal accroché, l'image arrive…"
                        else -> "En attente du satellite…"
                    },
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    "Antenne dehors (QFH ou V-dipôle), passage au-dessus de 20° d'élévation.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/** Right side in the LRPT space: downlink, decoding state, saving */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun LrptPanel(frequency: Double, l: LrptState, onTune: (Double) -> Unit, onOqpsk: (Boolean) -> Unit, onSave: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("Satellite", style = MaterialTheme.typography.titleSmall)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            LRPT_SATELLITES.forEach { s ->
                FilterChip(selected = kotlin.math.abs(frequency - s.hz) < 5_000, onClick = { onTune(s.hz) }, label = { Text(s.name) })
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("OQPSK", style = MaterialTheme.typography.bodyLarge)
                Text("Meteor-M N2-3 et N2-4 ; décoché : QPSK", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Switch(l.oqpsk, onOqpsk)
        }
        Text("Réception", style = MaterialTheme.typography.titleSmall)
        Stat("État", if (!l.active) "—" else if (l.locked) "Trames accrochées" else "Recherche")
        Stat("Porteuse", String.format(Locale.FRANCE, "%+.0f Hz", l.carrier))
        Stat("Trames", "${l.framesOk} bonnes · ${l.framesBad} ratées")
        Stat("Image", "${l.lines} lignes · canaux ${l.channels.joinToString().ifBlank { "—" }}")
        FilledTonalButton(onClick = onSave, enabled = l.packets > 0, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Default.Save, null, Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text("Enregistrer les images")
        }
    }
}

@Composable
private fun Stat(name: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(name, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}
