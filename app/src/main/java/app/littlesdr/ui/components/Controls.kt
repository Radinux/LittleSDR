package app.littlesdr.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.FlowRow
import kotlin.math.abs
import kotlinx.coroutines.launch
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.ui.graphics.Color
import androidx.compose.material3.IconButton
import androidx.compose.material.icons.filled.VolumeMute
import androidx.compose.material.icons.filled.SettingsInputAntenna
import androidx.compose.material.icons.filled.Bookmarks
import androidx.compose.material.icons.filled.LinearScale
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.Waves
import androidx.compose.material.icons.filled.Router
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.Straighten
import androidx.compose.material3.AssistChip
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.engine.Mode
import app.littlesdr.engine.SmAction
import app.littlesdr.engine.SmGui
import app.littlesdr.engine.SmWidget
import kotlinx.coroutines.flow.StateFlow
import app.littlesdr.ui.formatHz
import app.littlesdr.ui.theme.ReadoutStyle
import kotlin.math.roundToInt

@Composable
fun ModeSelector(selected: Mode?, onSelect: (Mode) -> Unit, modifier: Modifier = Modifier) {
    // Every mode visible (a scrolling row hid LSB/CW off the edge), in a reading order:
    // FM / AM family, then sidebands, CW and raw I/Q. The enum order is SDR++'s mode ids.
    FlowRow(
        modifier,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        MODE_ORDER.forEach { m ->
            FilterChip(
                selected = m == selected,
                onClick = { onSelect(m) },
                label = { Text(m.label) },
                leadingIcon = if (m == selected) {
                    { Icon(Icons.Default.Check, null, Modifier.size(FilterChipDefaults.IconSize)) }
                } else null,
            )
        }
    }
}

private val MODE_ORDER = listOf(Mode.NFM, Mode.WFM, Mode.AM, Mode.DSB, Mode.LSB, Mode.USB, Mode.CW, Mode.RAW)

private val STEPS = listOf(1.0, 10.0, 50.0, 100.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 6_250.0, 8_330.0, 9_000.0,
    10_000.0, 12_500.0, 25_000.0, 50_000.0, 100_000.0, 200_000.0, 1_000_000.0)

@Composable
fun StepChip(step: Double, onStep: (Double) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        AssistChip(
            onClick = { open = true },
            label = { Text("Pas ${formatHz(step)}") },
            leadingIcon = { Icon(Icons.Default.Straighten, null, Modifier.size(18.dp)) },
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            STEPS.forEach { s ->
                DropdownMenuItem(
                    text = { Text(formatHz(s)) },
                    onClick = { onStep(s); open = false },
                    trailingIcon = if (s == step) { { Icon(Icons.Default.Check, null) } } else null,
                )
            }
        }
    }
}

/** Slider that keeps its own value while dragging and only reports the final one. */
@Composable
private fun LabeledSlider(
    title: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    valueText: (Float) -> String,
    onChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leading: (@Composable () -> Unit)? = null,
    live: Boolean = false,
    snap: Float = 0f,
    titleExtra: (@Composable () -> Unit)? = null,
) {
    var dragging by remember { mutableStateOf(false) }
    var local by remember { mutableFloatStateOf(value) }
    val shown = if (dragging) local else value.coerceIn(range)
    Column(modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(title, style = MaterialTheme.typography.labelLarge)
            titleExtra?.let { Spacer(Modifier.width(8.dp)); it() }
            Spacer(Modifier.weight(1f))
            Text(valueText(shown), style = ReadoutStyle, color = MaterialTheme.colorScheme.primary)
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            leading?.invoke()
            Slider(
                value = shown,
                onValueChange = {
                    dragging = true
                    // Snap without Slider(steps=): hundreds of tick marks would clutter the track
                    val v = if (snap > 0f) ((it / snap).roundToInt() * snap).coerceIn(range) else it
                    // Live sliders only report actual changes (one engine call per snapped step)
                    if (live && v != local) onChange(v)
                    local = v
                },
                onValueChangeFinished = {
                    dragging = false
                    onChange(local)
                },
                valueRange = range,
                enabled = enabled,
                modifier = Modifier.weight(1f),
            )
        }
    }
}

/** Tabs of the control panel (top cap of the right "pill"). */
enum class ControlTab(val label: String) { Source("Récepteur"), Radio("Radio"), Audio("Audio"), Bands("Bandes"), Memories("Mémoires"), Morse("Morse") }

/**
 * Scrollable tab carousel: swipe sideways, edges fade out and tabs shrink as they approach them,
 * tapping a tab selects it and brings it to the middle. Scales to any number of tabs.
 */
@Composable
fun ControlTabBar(selected: ControlTab, onSelect: (ControlTab) -> Unit, modifier: Modifier = Modifier) {
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val tabs = ControlTab.entries
    val itemWidth = 84.dp
    val fade = 28.dp

    // Center the selected tab (on first show and after a tap)
    LaunchedEffect(selected) {
        val info = listState.layoutInfo
        val vw = info.viewportEndOffset - info.viewportStartOffset
        val iw = info.visibleItemsInfo.firstOrNull()?.size ?: 0
        if (vw > 0 && iw > 0) listState.animateScrollToItem(selected.ordinal, -(vw - iw) / 2)
    }

    LazyRow(
        state = listState,
        modifier = modifier
            .height(64.dp)
            // Edge fade: draw offscreen, then multiply by a horizontal alpha gradient
            .graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen)
            .drawWithContent {
                drawContent()
                val f = fade.toPx() / size.width
                drawRect(
                    Brush.horizontalGradient(
                        0f to Color.Transparent, f to Color.Black, 1f - f to Color.Black, 1f to Color.Transparent,
                    ),
                    blendMode = BlendMode.DstIn,
                )
            },
        contentPadding = PaddingValues(horizontal = fade),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        itemsIndexed(tabs, key = { _, t -> t.name }) { index, tab ->
            val isSel = tab == selected
            val icon = when (tab) {
                ControlTab.Source -> Icons.Default.Router
                ControlTab.Radio -> Icons.Default.Radio
                ControlTab.Audio -> Icons.AutoMirrored.Filled.VolumeUp
                ControlTab.Bands -> Icons.Default.Waves
                ControlTab.Memories -> Icons.Default.Bookmarks
                ControlTab.Morse -> Icons.Default.LinearScale
            }
            Column(
                Modifier
                    .width(itemWidth)
                    .height(56.dp)
                    // Shrink with the distance to the viewport center (read in the draw phase only)
                    .graphicsLayer {
                        val info = listState.layoutInfo
                        val item = info.visibleItemsInfo.firstOrNull { it.index == index }
                        if (item != null) {
                            val center = (info.viewportStartOffset + info.viewportEndOffset) / 2f
                            val half = (info.viewportEndOffset - info.viewportStartOffset) / 2f
                            val d = (abs(item.offset + item.size / 2f - center) / half).coerceIn(0f, 1f)
                            val k = 1f - 0.35f * d * d
                            scaleX = k
                            scaleY = k
                        }
                    }
                    .clip(RoundedCornerShape(18.dp))
                    .background(if (isSel) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                    .clickable {
                        onSelect(tab)
                        scope.launch {
                            val info = listState.layoutInfo
                            val vw = info.viewportEndOffset - info.viewportStartOffset
                            val iw = info.visibleItemsInfo.firstOrNull()?.size ?: 0
                            listState.animateScrollToItem(index, -(vw - iw) / 2)
                        }
                    },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                val tint = if (isSel) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onSurfaceVariant
                Icon(icon, null, Modifier.size(22.dp), tint = tint)
                Text(tab.label, style = MaterialTheme.typography.labelMedium, color = tint, maxLines = 1)
            }
        }
    }
}

@Composable
fun RadioTab(
    mode: Mode?,
    bandwidth: Double,
    autoBandwidth: Boolean,
    step: Double,
    signalDb: StateFlow<Float>,
    onMode: (Mode) -> Unit,
    onStep: (Double) -> Unit,
    onBandwidth: (Float) -> Unit,
    onAutoBandwidth: (Boolean) -> Unit,
    gain: List<SmWidget>,
    onGainAction: (SmAction) -> Unit,
    squelchEnabled: Boolean,
    squelchLevel: Float,
    onSquelch: (Boolean, Float) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        ModeSelector(mode, onMode, Modifier.fillMaxWidth())
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            StepChip(step, onStep)
            Spacer(Modifier.weight(1f))
            SignalMeter(signalDb, Modifier.width(140.dp))
        }
        val m = mode ?: Mode.NFM
        LabeledSlider(
            title = "Bande passante",
            value = bandwidth.toFloat(),
            range = m.bandwidthRange,
            valueText = { formatHz(it.toDouble()) },
            onChange = onBandwidth,
            enabled = m != Mode.RAW && !(autoBandwidth && m.isFm),
            snap = m.bandwidthStep,
            live = true,
            titleExtra = if (m.isFm) {
                {
                    FilterChip(
                        selected = autoBandwidth,
                        onClick = { onAutoBandwidth(!autoBandwidth) },
                        label = { Text("Auto") },
                        modifier = Modifier.height(28.dp),
                    )
                }
            } else null,
        )

        // Gain of the receiver (from the driver's own settings)
        SectionLabel("Gain")
        if (gain.isEmpty()) {
            Text("Pas de réglage de gain pour cette source.", style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            GainControls(gain, onGainAction)
        }

        // Squelch
        SectionLabel("Squelch")
        Row(verticalAlignment = Alignment.CenterVertically) {
            LabeledSlider(
                title = "Seuil",
                value = squelchLevel,
                range = -120f..0f,
                valueText = { "${it.roundToInt()} dB" },
                onChange = { onSquelch(squelchEnabled, it) },
                enabled = squelchEnabled,
                snap = 1f,
                live = true,
                modifier = Modifier.weight(1f),
            )
            Spacer(Modifier.width(12.dp))
            Switch(checked = squelchEnabled, onCheckedChange = { onSquelch(it, squelchLevel) })
        }
    }
}

@Composable
internal fun SectionLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(top = 10.dp),
    )
}

@Composable
fun AudioTab(
    volume: Float,
    onVolume: (Float) -> Unit,
    outputs: List<Pair<Int, String>>,
    device: Int,
    latency: Int,
    onDevice: (Int) -> Unit,
    onLatency: (Int) -> Unit,
) {
    // Mute keeps the previous level so un-muting restores it
    var lastVolume by remember { mutableFloatStateOf(if (volume > 0.001f) volume else 0.8f) }
    val muted = volume <= 0.001f
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        LabeledSlider(
            title = "Volume",
            value = volume,
            range = 0f..1f,
            valueText = { "${(it * 100).roundToInt()} %" },
            onChange = { if (it > 0.001f) lastVolume = it; onVolume(it) },
            live = true,
            leading = {
                IconButton(onClick = { onVolume(if (muted) lastVolume else 0f) }) {
                    Icon(
                        if (muted) Icons.AutoMirrored.Filled.VolumeOff else Icons.AutoMirrored.Filled.VolumeUp,
                        if (muted) "Remettre le son" else "Couper le son",
                    )
                }
            },
        )

        SectionLabel("Sortie")
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            val current = if (outputs.any { it.first == device }) device else 0
            outputs.forEach { (id, name) ->
                FilterChip(selected = id == current, onClick = { onDevice(id) }, label = { Text(name) })
            }
        }

        SectionLabel("Latence")
        val lat = when { latency <= 2 -> 2; latency >= 8 -> 8; else -> 4 }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf(2 to "Faible", 4 to "Normale", 8 to "Élevée").forEach { (v, label) ->
                FilterChip(selected = v == lat, onClick = { onLatency(v) }, label = { Text(label) })
            }
        }
        Text(
            "Faible : plus réactif, risque de coupures. Élevée : plus stable.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** Peak level in the VFO passband (dBFS, uncalibrated). */
@Composable
fun SignalMeter(signal: StateFlow<Float>, modifier: Modifier = Modifier) {
    val db by signal.collectAsStateWithLifecycle()
    val frac by animateFloatAsState(((db + 120f) / 100f).coerceIn(0f, 1f), label = "meter")
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.Default.GraphicEq, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.width(6.dp))
        Box(
            Modifier
                .weight(1f)
                .height(8.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
        ) {
            Box(
                Modifier
                    .fillMaxHeight()
                    .fillMaxWidth(frac)
                    .clip(RoundedCornerShape(4.dp))
                    .background(if (frac > 0.75f) MaterialTheme.colorScheme.secondary else MaterialTheme.colorScheme.primary),
            )
        }
        Spacer(Modifier.width(6.dp))
        Text(if (db <= -149f) "—" else "${db.roundToInt()} dB", style = ReadoutStyle, modifier = Modifier.width(52.dp))
    }
}


// R820T/R828D tuner gain steps (dB): the RTL-SDR module only exposes the step index (29 steps)
private val R820T_GAINS = floatArrayOf(
    0.0f, 0.9f, 1.4f, 2.7f, 3.7f, 7.7f, 8.7f, 12.5f, 14.4f, 15.7f, 16.6f, 19.7f, 20.7f, 22.9f, 25.4f,
    28.0f, 29.7f, 32.8f, 33.8f, 36.4f, 37.2f, 38.6f, 40.2f, 42.1f, 43.4f, 43.9f, 44.5f, 48.0f, 49.6f,
)

/**
 * Gain block built from the receiver's own settings (SmGui): every gain-like slider and AGC switch
 * of the current driver, so RTL-SDR, Airspy (LNA/mixer/VGA), HackRF, rtl_tcp... all get one.
 */
@Composable
fun GainControls(widgets: List<SmWidget>, onAction: (SmAction) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        val agc = widgets.filterIsInstance<SmWidget.Checkbox>()
        widgets.forEach { w ->
            when (w) {
                is SmWidget.SliderInt -> {
                    val r820 = w.fmt == "none" && w.max - w.min == R820T_GAINS.size - 1
                    LabeledSlider(
                        title = w.title ?: w.label.ifBlank { "Gain" },
                        value = w.value.toFloat(),
                        range = w.min.toFloat()..w.max.toFloat().coerceAtLeast(w.min + 1f),
                        valueText = { v ->
                            val i = v.roundToInt()
                            when {
                                r820 -> "${R820T_GAINS[(i - w.min).coerceIn(0, R820T_GAINS.size - 1)]} dB"
                                w.fmt == "db" -> "$i dB"
                                else -> "$i"
                            }
                        },
                        onChange = { onAction(SmGui.int(w.id, it.roundToInt())) },
                        enabled = !w.disabled,
                        snap = 1f,
                        live = true,
                    )
                }
                is SmWidget.SliderFloat -> LabeledSlider(
                    title = w.title ?: w.label.ifBlank { "Gain" },
                    value = w.value,
                    range = w.min..w.max.coerceAtLeast(w.min + 0.001f),
                    valueText = { v -> String.format(java.util.Locale.ROOT, "%.${w.decimals.coerceIn(0, 2)}f", v) + if (w.fmt == "db") " dB" else "" },
                    onChange = { onAction(SmGui.float(w.id, it)) },
                    enabled = !w.disabled,
                    snap = if (w.step > 0f) w.step else 0f,
                    live = true,
                )
                else -> Unit
            }
        }
        if (agc.isNotEmpty()) {
            Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                agc.forEach { c ->
                    FilterChip(
                        selected = c.value,
                        onClick = { onAction(SmGui.bool(c.id, !c.value)) },
                        label = { Text(c.label) },
                        enabled = !c.disabled,
                    )
                }
            }
        }
    }
}
