package app.littlesdr.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.littlesdr.engine.Converter
import app.littlesdr.engine.DemodOptions
import app.littlesdr.engine.FftOptions
import app.littlesdr.engine.InputOptions
import app.littlesdr.ui.theme.ReadoutStyle
import java.util.Locale
import kotlin.math.roundToInt

private typealias SetNum = (String, Double) -> Unit
private typealias SetBool = (String, Boolean) -> Unit

// ---------------------------------------------------------------- building blocks

@Composable
private fun OptionCard(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 4.dp))
        Card(
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
            shape = androidx.compose.foundation.shape.RoundedCornerShape(24.dp),
        ) {
            Column(Modifier.padding(vertical = 8.dp)) { content() }
        }
    }
}

@Composable
private fun SwitchRow(title: String, subtitle: String?, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = subtitle?.let { { Text(it) } },
        trailingContent = { Switch(checked = checked, onCheckedChange = onChange, enabled = enabled) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.padding(horizontal = 4.dp),
    )
}

@Composable
private fun <T> ChoiceRow(title: String, options: List<Pair<T, String>>, selected: T, enabled: Boolean = true, onSelect: (T) -> Unit) {
    Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            options.forEach { (v, label) ->
                FilterChip(selected = v == selected, onClick = { onSelect(v) }, label = { Text(label) }, enabled = enabled)
            }
        }
    }
}

/** Slider committing on release (engine calls can be costly, e.g. FFT reconfiguration). */
@Composable
private fun SliderRow(
    title: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    text: (Float) -> String,
    enabled: Boolean = true,
    steps: Int = 0,
    onDone: (Float) -> Unit,
) {
    var dragging by remember { mutableStateOf(false) }
    var local by remember { mutableFloatStateOf(value) }
    val shown = if (dragging) local else value.coerceIn(range)
    Column(Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
            Text(text(shown), style = ReadoutStyle, color = MaterialTheme.colorScheme.primary)
        }
        Slider(
            value = shown,
            onValueChange = { dragging = true; local = it },
            onValueChangeFinished = { dragging = false; onDone(local) },
            valueRange = range,
            steps = steps,
            enabled = enabled,
        )
    }
}

@Composable
private fun Divider() = HorizontalDivider(Modifier.padding(horizontal = 20.dp, vertical = 6.dp))

// ---------------------------------------------------------------- Demodulator

@Composable
fun DemodSection(d: DemodOptions, setNum: SetNum, setBool: SetBool) {
    OptionCard(if (d.demod.isBlank()) "Démodulateur" else "Démodulateur · ${d.demod}") {
        var any = false
        if (d.deempAllowed && d.deempModes.isNotEmpty()) {
            any = true
            ChoiceRow(
                "Désaccentuation",
                d.deempModes.mapIndexed { i, name -> i to if (name == "None") "Aucune" else name.replace("us", " µs") },
                d.deemp,
            ) { setNum("demod.deemp", it.toDouble()) }
        }
        if (d.stereoAllowed) {
            if (any) Divider()
            any = true
            SwitchRow("Stéréo", "Décode le son stéréo des radios FM (sinon mono, moins de souffle)", d.stereo) { setBool("demod.stereo", it) }
        }
        if (d.ifnrAllowed) {
            if (any) Divider()
            any = true
            SwitchRow("Réduction de bruit FM", "Filtre adaptatif sur le signal FM (IF NR)", d.ifnr) { setBool("demod.ifnr", it) }
            if (d.ifnr && d.ifnrPresets.isNotEmpty()) {
                val labels = mapOf("NOAA APT" to "NOAA APT", "Voice" to "Voix", "Narrow Band" to "Bande étroite")
                ChoiceRow("Préréglage", d.ifnrPresets.mapIndexed { i, n -> i to (labels[n] ?: n) }, d.ifnrPreset) {
                    setNum("demod.ifnrPreset", it.toDouble())
                }
            }
        }
        if (d.nbAllowed) {
            if (any) Divider()
            any = true
            SwitchRow("Noise blanker", "Supprime les parasites impulsionnels", d.nb) { setBool("demod.nb", it) }
            if (d.nb) {
                SliderRow("Niveau", d.nbLevel, 1f..10f, { String.format(Locale.ROOT, "%.1f", it) }) {
                    setNum("demod.nbLevel", it.toDouble())
                }
            }
        }
        if (d.agc) {
            if (any) Divider()
            any = true
            SliderRow("AGC · attaque", d.agcAttack, 1f..200f, { "${it.roundToInt()}" }) { setNum("demod.agcAttack", it.toDouble()) }
            SliderRow("AGC · déclin", d.agcDecay, 1f..20f, { String.format(Locale.ROOT, "%.1f", it) }) { setNum("demod.agcDecay", it.toDouble()) }
        }
        if (!any) {
            Text(
                "Aucune option pour ce mode.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp),
            )
        }
    }
}

// ---------------------------------------------------------------- Input / source

@Composable
fun InputSection(i: InputOptions, playing: Boolean, setNum: SetNum, setBool: SetBool) {
    OptionCard("Entrée") {
        ChoiceRow(
            "Décimation",
            listOf(1, 2, 4, 8, 16, 32, 64).map { it to if (it == 1) "Aucune" else "${it}×" },
            i.decimation,
            enabled = !playing,
        ) { setNum("input.decimation", it.toDouble()) }
        if (playing) {
            Text("Modifiable à l'arrêt", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 20.dp))
        }
        Divider()
        SwitchRow("Correction du DC", "Retire la raie centrale du récepteur", i.dcBlocking) { setBool("input.dcBlocking", it) }
        SwitchRow("Inverser I/Q", "Spectre inversé (certains récepteurs, mélangeurs)", i.invertIQ) { setBool("input.invertIQ", it) }
        Divider()

        // Frequency converter
        val conv = Converter.of(i.offset)
        var manual by remember { mutableStateOf(conv == Converter.Manual) }
        val selected = if (manual) Converter.Manual else conv
        ChoiceRow("Convertisseur de fréquence", Converter.entries.map { it to it.label }, selected) { c ->
            manual = c == Converter.Manual
            if (!manual) setNum("input.offset", c.offsetHz)
        }
        if (selected == Converter.Manual) {
            var text by remember(i.offset) { mutableStateOf(String.format(Locale.ROOT, "%.6f", i.offset / 1e6).trimEnd('0').trimEnd('.')) }
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text("Décalage (MHz)") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = {
                    text.replace(',', '.').toDoubleOrNull()?.let { setNum("input.offset", it * 1e6) }
                }),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp),
            )
        }
    }
}

// ---------------------------------------------------------------- FFT

@Composable
fun FftSection(f: FftOptions, setNum: SetNum, setBool: SetBool) {
    OptionCard("FFT") {
        ChoiceRow(
            "Résolution",
            listOf(1024, 2048, 4096, 8192, 16384, 32768, 65536).map { it to if (it >= 1024) "${it / 1024}k" else "$it" },
            f.size,
        ) { setNum("fft.size", it.toDouble()) }
        SliderRow("Images par seconde", f.rate, 5f..60f, { "${it.roundToInt()}" }) { setNum("fft.rate", it.roundToInt().toDouble()) }
        ChoiceRow("Fenêtre", listOf(0 to "Rectangulaire", 1 to "Blackman", 2 to "Nuttall"), f.window) {
            setNum("fft.window", it.toDouble())
        }
        Divider()
        SliderRow("Lissage de la courbe", f.smoothing, 0f..0.9f, { "${(it * 100).roundToInt()} %" }) { setNum("fft.smoothing", it.toDouble()) }
        SwitchRow("Maintien des crêtes", "Trace la valeur maximale récente", f.peakHold) { setBool("fft.peakHold", it) }
        if (f.peakHold) {
            SliderRow("Retombée des crêtes", f.peakDecay, 1f..40f, { "${it.roundToInt()} dB/s" }) { setNum("fft.peakDecay", it.toDouble()) }
        }
    }
}

// ---------------------------------------------------------------- Operator

@Composable
fun OperatorSection(
    o: app.littlesdr.engine.OperatorOptions,
    autoLocator: Boolean,
    locating: Boolean,
    setText: (String, String) -> Unit,
    onAutoLocator: (Boolean) -> Unit,
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val permission = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestPermission(),
    ) { granted -> if (granted) onAutoLocator(true) }

    OptionCard("Opérateur") {
        var call by remember(o.callsign) { mutableStateOf(o.callsign) }
        var loc by remember(o.location) { mutableStateOf(o.location) }
        OutlinedTextField(
            value = call,
            onValueChange = { call = it.uppercase().filter { c -> c.isLetterOrDigit() || c == '/' }.take(12) },
            label = { Text("Indicatif") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { setText("operator.callsign", call) }),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp),
        )
        SwitchRow(
            "Locator automatique",
            if (locating) "Recherche de la position…" else "D'après la position approximative du téléphone",
            autoLocator,
        ) { on ->
            if (on && !app.littlesdr.data.PhoneLocation.hasPermission(context)) {
                permission.launch(android.Manifest.permission.ACCESS_COARSE_LOCATION)
            } else {
                onAutoLocator(on)
            }
        }
        OutlinedTextField(
            value = loc,
            onValueChange = { loc = it.uppercase().filter { c -> c.isLetterOrDigit() }.take(8) },
            label = { Text("Locator (ex. JN18)") },
            supportingText = { Text("Sert à calculer la distance des stations FT8") },
            singleLine = true,
            readOnly = autoLocator,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { setText("operator.location", loc) }),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 4.dp),
        )
    }
}


// ---------------------------------------------------------------- EiBi

@Composable
fun EiBiSection(season: String, count: Int, updated: Long, updating: Boolean, onUpdate: () -> Unit) {
    OptionCard("Horaires ondes courtes (EiBi)") {
        val date = if (updated > 0) java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(updated)) else null
        ListItem(
            headlineContent = { Text(if (count > 0) "Saison $season · $count émissions" else "Pas encore téléchargé") },
            supportingContent = {
                Text(
                    (date?.let { "Mis à jour le $it. " } ?: "") +
                        "Affiche dans l'encoche les stations qui émettent sur la fréquence en ce moment (hors FM). Données eibispace.de.",
                )
            },
            trailingContent = {
                if (updating) androidx.compose.material3.CircularProgressIndicator(Modifier.padding(8.dp))
                else androidx.compose.material3.FilledTonalButton(onClick = onUpdate) { Text("Mettre à jour") }
            },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            modifier = Modifier.padding(horizontal = 4.dp),
        )
    }
}
