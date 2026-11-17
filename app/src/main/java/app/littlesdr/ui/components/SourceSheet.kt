package app.littlesdr.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.ui.draw.clip
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.SmAction
import app.littlesdr.engine.SmGui
import app.littlesdr.engine.SmWidget
import app.littlesdr.ui.theme.ReadoutStyle
import app.littlesdr.usb.UsbSdrInfo
import java.util.Locale
import kotlin.math.roundToInt

/** Receiver tab: source choice, connected USB device and the driver's own settings (SmGui). */
@Composable
fun SourcePanel(
    sources: List<String>,
    selected: String,
    playing: Boolean,
    usbDevice: UsbSdrInfo?,
    menu: List<SmWidget>,
    onSelect: (String) -> Unit,
    onAction: (SmAction) -> Unit,
    onWatch: (Boolean) -> Unit,
) {
    // Faster refresh of the driver menu while this tab is shown (status texts, device lists)
    DisposableEffect(Unit) {
        onWatch(true)
        onDispose { onWatch(false) }
    }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        // Source choice as rounded pills (USB receivers vs network sources)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            sources.forEach { name ->
                val isSel = name == selected
                val network = name in NETWORK_SOURCES
                Row(
                    Modifier
                        .clip(RoundedCornerShape(50))
                        .background(if (isSel) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainer)
                        .clickable(enabled = !playing && !isSel) { onSelect(name) }
                        .padding(horizontal = 14.dp, vertical = 9.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        if (network) Icons.Default.Wifi else Icons.Default.Usb, null, Modifier.size(18.dp),
                        tint = if (isSel) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.width(6.dp))
                    Text(
                        name,
                        style = MaterialTheme.typography.labelLarge,
                        color = when {
                            isSel -> MaterialTheme.colorScheme.onPrimaryContainer
                            playing -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
                            else -> MaterialTheme.colorScheme.onSurface
                        },
                    )
                }
            }
        }
        if (playing) {
            Text(
                "Arrête la réception pour changer de récepteur ou modifier certains réglages.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (usbDevice != null) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(20.dp))
                    .background(MaterialTheme.colorScheme.secondaryContainer)
                    .padding(horizontal = 14.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Default.Usb, null, tint = MaterialTheme.colorScheme.onSecondaryContainer)
                Spacer(Modifier.width(10.dp))
                Column {
                    Text(usbDevice.name, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSecondaryContainer)
                    Text("Récepteur USB connecté", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSecondaryContainer)
                }
            }
        }
        if (menu.isNotEmpty()) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(20.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainer)
                    .padding(14.dp),
            ) {
                SmGuiPanel(menu, onAction)
            }
        }
    }
}

private val NETWORK_SOURCES = setOf("RTL-TCP", "SpyServer", "KiwiSDR", "SDR++ Server")

/** Material 3 rendering of an SDR++ SmGui menu. Consecutive "same line" widgets share a row. */
@Composable
fun SmGuiPanel(widgets: List<SmWidget>, onAction: (SmAction) -> Unit) {
    // Group into rows
    val rows = mutableListOf<MutableList<SmWidget>>()
    widgets.forEach { w ->
        if (w.sameLine && rows.isNotEmpty() && w !is SmWidget.Header) rows.last().add(w) else rows.add(mutableListOf(w))
    }
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        rows.forEach { row ->
            if (row.size == 1) {
                SmWidgetView(row[0], onAction, Modifier.fillMaxWidth())
            } else {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    row.forEach { SmWidgetView(it, onAction) }
                }
            }
        }
    }
}

@Composable
private fun SmWidgetView(w: SmWidget, onAction: (SmAction) -> Unit, modifier: Modifier = Modifier) {
    val enabled = !w.disabled
    when (w) {
        is SmWidget.Header -> Text(w.value, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary, modifier = modifier.padding(top = 8.dp))
        is SmWidget.Text -> Text(
            listOfNotNull(w.title, w.value).joinToString(" "),
            style = MaterialTheme.typography.bodyMedium,
            color = w.color?.let { Color(it) } ?: MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = modifier,
        )
        is SmWidget.Button -> FilledTonalButton(onClick = { onAction(SmGui.click(w.id)) }, enabled = enabled, modifier = modifier) {
            Text(w.label.ifBlank { w.title ?: "OK" })
        }
        is SmWidget.Checkbox -> Row(modifier, verticalAlignment = Alignment.CenterVertically) {
            Text(w.label.ifBlank { w.title ?: "" }, Modifier.weight(1f, fill = false).padding(end = 12.dp))
            Spacer(Modifier.weight(1f))
            Switch(checked = w.value, onCheckedChange = { onAction(SmGui.bool(w.id, it)) }, enabled = enabled)
        }
        is SmWidget.Radio -> Row(modifier, verticalAlignment = Alignment.CenterVertically) {
            RadioButton(selected = w.value, onClick = { onAction(SmGui.click(w.id)) }, enabled = enabled)
            Text(w.label)
        }
        is SmWidget.Combo -> Dropdown(
            label = w.title ?: w.label.ifBlank { "" },
            items = w.items,
            selected = w.value,
            enabled = enabled,
            onSelect = { onAction(SmGui.int(w.id, it)) },
            modifier = modifier,
        )
        is SmWidget.SliderInt -> SmSlider(
            title = w.title ?: w.label, value = w.value.toFloat(), min = w.min.toFloat(), max = w.max.toFloat(),
            steps = (w.max - w.min - 1).coerceIn(0, 200), enabled = enabled, modifier = modifier,
            text = { v ->
                val i = v.roundToInt()
                when (w.fmt) { "db" -> "$i dB"; "percent" -> "+$i %"; "none" -> "${i - w.min + 1}/${w.max - w.min + 1}"; else -> "$i" }
            },
            onDone = { onAction(SmGui.int(w.id, it.roundToInt())) },
        )
        is SmWidget.SliderFloat -> SmSlider(
            title = w.title ?: w.label, value = w.value, min = w.min, max = w.max,
            steps = if (w.step > 0) (((w.max - w.min) / w.step).roundToInt() - 1).coerceIn(0, 200) else 0,
            enabled = enabled, modifier = modifier,
            text = { v -> String.format(Locale.ROOT, "%.${w.decimals}f", v) + if (w.fmt == "db") " dB" else "" },
            onDone = { onAction(SmGui.float(w.id, it)) },
        )
        is SmWidget.InputInt -> Row(modifier, verticalAlignment = Alignment.CenterVertically) {
            Text(w.title ?: w.label, Modifier.weight(1f))
            FilledTonalIconButton(onClick = { onAction(SmGui.int(w.id, w.value - w.step.coerceAtLeast(1))) }, enabled = enabled) { Icon(Icons.Default.Remove, "−") }
            Text("${w.value}", style = ReadoutStyle, modifier = Modifier.width(64.dp), textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            FilledTonalIconButton(onClick = { onAction(SmGui.int(w.id, w.value + w.step.coerceAtLeast(1))) }, enabled = enabled) { Icon(Icons.Default.Add, "+") }
        }
        is SmWidget.InputText -> {
            var text by remember(w.id, w.value) { mutableStateOf(w.value) }
            TextField(
                value = text,
                onValueChange = { if (it.length <= w.maxLen.coerceAtLeast(1)) text = it },
                label = { Text(w.title ?: w.label.ifBlank { "Valeur" }) },
                singleLine = true,
                enabled = enabled,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, keyboardType = KeyboardType.Uri),
                keyboardActions = KeyboardActions(onDone = { onAction(SmGui.text(w.id, text)) }),
                shape = RoundedCornerShape(16.dp),
                colors = roundedFieldColors(),
                modifier = modifier,
            )
        }
    }
}

@Composable
private fun SmSlider(
    title: String, value: Float, min: Float, max: Float, steps: Int, enabled: Boolean,
    text: (Float) -> String, onDone: (Float) -> Unit, modifier: Modifier = Modifier,
) {
    var dragging by remember { mutableStateOf(false) }
    var local by remember { mutableFloatStateOf(value) }
    val shown = if (dragging) local else value.coerceIn(min, max.coerceAtLeast(min))
    Column(modifier) {
        Row {
            Text(title, style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
            Text(text(shown), style = ReadoutStyle, color = MaterialTheme.colorScheme.primary)
        }
        Slider(
            value = shown,
            onValueChange = { dragging = true; local = it },
            onValueChangeFinished = { dragging = false; onDone(local) },
            valueRange = min..max.coerceAtLeast(min + 0.0001f),
            steps = steps,
            enabled = enabled,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Dropdown(
    label: String,
    items: List<String>,
    selected: Int,
    enabled: Boolean,
    onSelect: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    var expanded by remember { mutableStateOf(false) }
    ExposedDropdownMenuBox(expanded = expanded && enabled, onExpandedChange = { if (enabled) expanded = it }, modifier = modifier) {
        TextField(
            value = items.getOrNull(selected) ?: "",
            onValueChange = {},
            readOnly = true,
            enabled = enabled,
            singleLine = true,
            label = if (label.isNotBlank()) { { Text(label) } } else null,
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) },
            shape = RoundedCornerShape(16.dp),
            colors = roundedFieldColors(),
            modifier = Modifier
                .menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable, enabled)
                .fillMaxWidth(),
        )
        ExposedDropdownMenu(expanded = expanded && enabled, onDismissRequest = { expanded = false }) {
            items.forEachIndexed { i, item ->
                DropdownMenuItem(text = { Text(item) }, onClick = { onSelect(i); expanded = false })
            }
        }
    }
}


/** Filled field without the underline: reads as a rounded tonal box. */
@Composable
private fun roundedFieldColors() = TextFieldDefaults.colors(
    focusedIndicatorColor = Color.Transparent,
    unfocusedIndicatorColor = Color.Transparent,
    disabledIndicatorColor = Color.Transparent,
    focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
    disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
)
