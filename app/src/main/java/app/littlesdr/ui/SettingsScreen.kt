package app.littlesdr.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RangeSlider
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.data.Appearance
import app.littlesdr.data.DisplaySettings
import app.littlesdr.ui.theme.Accent
import app.littlesdr.ui.theme.Palette
import app.littlesdr.ui.theme.ReadoutStyle
import app.littlesdr.ui.theme.dynamicColorAvailable
import kotlin.math.roundToInt

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(vm: RadioViewModel, onBack: () -> Unit) {
    val data by vm.data.collectAsStateWithLifecycle()
    val appearance = data.appearance
    val display = data.display
    val adv by vm.advanced.collectAsStateWithLifecycle()
    val playing by remember { derivedStateOf { vm.state.value.playing } }
    LaunchedEffect(Unit) { vm.refreshAdvanced() }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Réglages") },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Retour") }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
            )
        },
        containerColor = MaterialTheme.colorScheme.surface,
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.TopCenter) {
            // Landscape: two columns, appearance + about | spectrum & waterfall
            Row(
                Modifier
                    .widthIn(max = 1200.dp)
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .windowInsetsPadding(WindowInsets.navigationBars)
                    .padding(horizontal = 24.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    AppearanceSection(appearance) { vm.setAppearance(it) }
                    SpectrumSection(display) { vm.setDisplay(it) }
                    FftSection(adv.fft, vm::setOption, vm::setOption)
                    Spacer(Modifier.height(16.dp))
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    DemodSection(adv.demod, vm::setOption, vm::setOption)
                    InputSection(adv.input, playing, vm::setOption, vm::setOption)
                    AudioSection(adv.audio, remember { vm.audioOutputs() }, vm::setOption)
                    val locating by vm.locating.collectAsStateWithLifecycle()
                    OperatorSection(adv.operator, data.operator.autoLocator, locating, vm::setOptionString, vm::setAutoLocator)
                    AboutSection()
                    Spacer(Modifier.height(16.dp))
                }
            }
        }
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 4.dp))
        Card(
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
            shape = RoundedCornerShape(24.dp),
        ) {
            Column(Modifier.padding(vertical = 8.dp)) { content() }
        }
    }
}

@Composable
private fun SettingLabel(title: String, subtitle: String? = null, modifier: Modifier = Modifier) {
    Column(modifier) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        if (subtitle != null) {
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

// ---------------------------------------------------------------- Appearance

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppearanceSection(a: Appearance, onChange: (Appearance) -> Unit) {
    Section("Apparence") {
        // Theme mode
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SettingLabel("Thème")
            val modes = listOf("Système", "Sombre", "Clair")
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                modes.forEachIndexed { i, label ->
                    SegmentedButton(
                        selected = a.themeMode == i,
                        onClick = { onChange(a.copy(themeMode = i)) },
                        shape = SegmentedButtonDefaults.itemShape(i, modes.size),
                    ) { Text(label) }
                }
            }
        }
        HorizontalDivider(Modifier.padding(horizontal = 20.dp, vertical = 8.dp))

        // Material You
        ListItem(
            headlineContent = { Text("Couleurs dynamiques") },
            supportingContent = {
                Text(if (dynamicColorAvailable) "Couleurs tirées du fond d'écran (Material You)" else "Nécessite Android 12 ou plus")
            },
            trailingContent = {
                Switch(
                    checked = a.dynamicColor && dynamicColorAvailable,
                    onCheckedChange = { onChange(a.copy(dynamicColor = it)) },
                    enabled = dynamicColorAvailable,
                )
            },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            modifier = Modifier.padding(horizontal = 4.dp),
        )

        // Accent color
        val accentEnabled = !(a.dynamicColor && dynamicColorAvailable)
        Column(
            Modifier.padding(horizontal = 20.dp, vertical = 8.dp).alpha(if (accentEnabled) 1f else 0.38f),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SettingLabel("Couleur d'accent", if (accentEnabled) null else "Désactive les couleurs dynamiques pour choisir")
            FlowRow(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Accent.entries.forEach { accent ->
                    val selected = a.accent == accent.ordinal
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Box(
                            Modifier
                                .size(48.dp)
                                .clip(CircleShape)
                                .background(Color(accent.primary))
                                .border(
                                    width = if (selected) 3.dp else 0.dp,
                                    color = if (selected) MaterialTheme.colorScheme.onSurface else Color.Transparent,
                                    shape = CircleShape,
                                )
                                .selectable(
                                    selected = selected,
                                    enabled = accentEnabled,
                                    role = Role.RadioButton,
                                    onClick = { onChange(a.copy(accent = accent.ordinal)) },
                                ),
                            contentAlignment = Alignment.Center,
                        ) {
                            if (selected) Icon(Icons.Default.Check, null, tint = Color(accent.onPrimary))
                        }
                        Spacer(Modifier.height(4.dp))
                        Text(accent.label, style = MaterialTheme.typography.labelSmall)
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- Spectrum & waterfall

@Composable
private fun SpectrumSection(d: DisplaySettings, onChange: (DisplaySettings) -> Unit) {
    Section("Spectre et waterfall") {
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            SettingLabel("Palette du waterfall")
            FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Palette.entries.forEach { p ->
                    PaletteTile(p, selected = d.palette == p.ordinal) { onChange(d.copy(palette = p.ordinal)) }
                }
            }
        }
        HorizontalDivider(Modifier.padding(horizontal = 20.dp, vertical = 8.dp))

        // dB range (applied when the finger lifts: avoids rewriting the store on every move)
        var range by remember(d.dbMin, d.dbMax) { mutableStateOf(d.dbMin..d.dbMax) }
        Column(Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SettingLabel("Plage d'affichage", "Bas et haut de l'échelle du spectre et des couleurs", Modifier.weight(1f))
                Text("${range.start.roundToInt()} … ${range.endInclusive.roundToInt()} dB", style = ReadoutStyle,
                    color = MaterialTheme.colorScheme.primary)
            }
            RangeSlider(
                value = range,
                onValueChange = { if (it.endInclusive - it.start >= 10f) range = it },
                onValueChangeFinished = { onChange(d.copy(dbMin = range.start, dbMax = range.endInclusive)) },
                valueRange = -160f..0f,
            )
        }

        var fraction by remember(d.spectrumFraction) { mutableFloatStateOf(d.spectrumFraction) }
        Column(Modifier.padding(horizontal = 20.dp, vertical = 4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SettingLabel("Hauteur du spectre", "Part de la courbe au-dessus du waterfall", Modifier.weight(1f))
                Text("${(fraction * 100).roundToInt()} %", style = ReadoutStyle, color = MaterialTheme.colorScheme.primary)
            }
            Slider(
                value = fraction,
                onValueChange = { fraction = it },
                onValueChangeFinished = { onChange(d.copy(spectrumFraction = fraction)) },
                valueRange = 0.15f..0.7f,
            )
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp), horizontalArrangement = Arrangement.End) {
            TextButton(onClick = { onChange(DisplaySettings(palette = d.palette)) }) { Text("Valeurs par défaut") }
        }
    }
}

@Composable
private fun PaletteTile(p: Palette, selected: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    Column(
        Modifier
            .width(150.dp)
            .clip(shape)
            .border(
                width = if (selected) 2.dp else 1.dp,
                color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant,
                shape = shape,
            )
            .selectable(selected = selected, role = Role.RadioButton, onClick = onClick)
            .padding(8.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(28.dp)
                .clip(RoundedCornerShape(10.dp))
                .background(Brush.horizontalGradient(p.stops())),
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(p.label, style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
            if (selected) Icon(Icons.Default.Check, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
        }
    }
}

// ---------------------------------------------------------------- About

@Composable
private fun AboutSection() {
    val ctx = LocalContext.current
    val version = remember {
        runCatching { ctx.packageManager.getPackageInfo(ctx.packageName, 0).versionName }.getOrNull() ?: "?"
    }
    Section("À propos") {
        ListItem(
            headlineContent = { Text("LittleSDR $version") },
            supportingContent = { Text("Interface Jetpack Compose Material 3, rendu du spectre en Vulkan") },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )
        ListItem(
            headlineContent = { Text("Moteur radio : SDR++Brown") },
            supportingContent = { Text("Traitement du signal et pilotes de SDR++Brown (basé sur SDR++), licence GPL-3.0") },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )
    }
}
