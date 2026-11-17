package app.littlesdr.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
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
import androidx.compose.material.icons.filled.Bookmarks
import androidx.compose.material.icons.filled.Forum
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Router
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import app.littlesdr.engine.Mode
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.engine.RadioState
import app.littlesdr.ui.components.DemodControls
import app.littlesdr.ui.components.FrequencyDisplay
import app.littlesdr.ui.components.MemoriesSheet
import app.littlesdr.ui.components.SourceSheet
import app.littlesdr.ui.components.SpectrumPanel
import app.littlesdr.ui.components.TuningJoystick
import app.littlesdr.ui.theme.ReadoutStyle
import kotlinx.coroutines.launch

private enum class Sheet { None, Source, Memories }

@Composable
fun MainScreen(vm: RadioViewModel, onOpenSettings: () -> Unit, onOpenFt8: () -> Unit) {
    val ready by vm.ready.collectAsStateWithLifecycle()
    val stateHolder = vm.state.collectAsStateWithLifecycle()
    val viewHolder = vm.view.collectAsStateWithLifecycle()
    // Fine-grained reads: tuning (frequency changes) must not recompose the whole screen
    val playing by remember { derivedStateOf { stateHolder.value.playing } }
    val mode by remember { derivedStateOf { stateHolder.value.mode } }
    val controls by remember {
        derivedStateOf {
            stateHolder.value.let { ControlValues(it.mode, it.vfoBandwidth, it.squelchEnabled, it.squelchLevel, it.volume) }
        }
    }
    val data by vm.data.collectAsStateWithLifecycle()
    val sources by vm.sources.collectAsStateWithLifecycle()
    val selectedSource by vm.selectedSource.collectAsStateWithLifecycle()
    val usb by vm.usbDevice.collectAsStateWithLifecycle()
    val menu by vm.sourceMenu.collectAsStateWithLifecycle()
    val gainControls by vm.gainControls.collectAsStateWithLifecycle()
    val ft8State by vm.ft8.collectAsStateWithLifecycle()
    val ft8Active = ft8State.enabled
    val error by vm.error.collectAsStateWithLifecycle()
    vm.steps.collectAsStateWithLifecycle()
    val step = vm.stepFor(mode)

    var sheet by rememberSaveable { mutableStateOf(Sheet.None) }

    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    // Renderer colors follow the Material theme (dynamic color included)
    val cs = MaterialTheme.colorScheme
    LaunchedEffect(cs.primary, cs.surfaceContainerLowest) {
        vm.setDisplayColors(cs.primary.copy(alpha = 0.35f).toArgb(), cs.primary.toArgb(), cs.surfaceContainerLowest.toArgb())
    }
    LaunchedEffect(error) {
        error?.let { snackbar.showSnackbar(it); vm.clearError() }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = MaterialTheme.colorScheme.surface,
    ) { _ ->
        Box(Modifier.fillMaxSize()) {
            // Landscape only: spectrum + waterfall on the left, control panel on the right.
            // Margins keep everything clear of rounded screen corners, cutouts and the nav bar.
            val corners = rememberScreenCorners()
            val m = corners.margin
            Row(
                Modifier
                    .fillMaxSize()
                    .windowInsetsPadding(WindowInsets.safeDrawing.union(WindowInsets(m, m, m, m))),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Column(
                    Modifier.weight(1f).fillMaxHeight(),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    SpectrumPanel(
                        state = stateHolder, view = viewHolder, display = data.display, step = step,
                        onTune = vm::tune, onView = vm::setView, onResetZoom = vm::resetZoom,
                        cornerRadius = corners.innerRadius,
                        maskColor = MaterialTheme.colorScheme.surface,
                        modifier = Modifier.weight(1f).fillMaxWidth(),
                    )
                    // Rate joystick: deflection = tuning speed, springs back to stop
                    TuningJoystick(
                        frequency = { stateHolder.value.vfoFrequency },
                        span = { viewHolder.value.bandwidth.takeIf { it > 0 } ?: stateHolder.value.sampleRate },
                        step = { vm.stepFor(stateHolder.value.mode) },
                        onTune = vm::tune,
                        modifier = Modifier.fillMaxWidth().height(52.dp),
                    )
                }
                Surface(
                    color = MaterialTheme.colorScheme.surfaceContainerLow,
                    shape = RoundedCornerShape(corners.innerRadius),
                    modifier = Modifier.width(360.dp).fillMaxHeight(),
                ) {
                    Column(
                        Modifier
                            .fillMaxSize()
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 14.dp, vertical = 10.dp),
                    ) {
                        TopRow(
                            source = selectedSource,
                            usb = usb != null,
                            playing = playing,
                            playEnabled = ready,
                            onPlay = vm::togglePlay,
                            onSource = { sheet = Sheet.Source },
                            onMemories = { sheet = Sheet.Memories },
                            onSettings = onOpenSettings,
                            onFt8 = onOpenFt8,
                            ft8Active = ft8Active,
                        )
                        Header(stateHolder, onTune = vm::tune)
                        Spacer(Modifier.height(6.dp))
                        DemodControls(
                            mode = controls.mode,
                            bandwidth = controls.bandwidth,
                            squelchEnabled = controls.squelchEnabled,
                            squelchLevel = controls.squelchLevel,
                            volume = controls.volume,
                            autoBandwidth = data.radio.autoBandwidth,
                            gain = gainControls,
                            onGainAction = { vm.sourceAction(it) },
                            signalDb = vm.signalDb, step = step,
                            onMode = vm::setMode,
                            onStep = { s -> mode?.let { vm.setStep(it, s) } },
                            onBandwidth = vm::setBandwidth,
                            onAutoBandwidth = { vm.setAutoBandwidth(it) },
                            onSquelch = vm::setSquelch,
                            onVolume = vm::setVolume,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
            }

            if (!ready) {
                Box(
                    Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.5f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        CircularProgressIndicator()
                        Spacer(Modifier.height(12.dp))
                        Text("Démarrage du moteur…", color = MaterialTheme.colorScheme.onSurface)
                    }
                }
            }
        }
    }

    when (sheet) {
        Sheet.Source -> SourceSheet(
            sources = sources, selected = selectedSource, playing = playing, usbDevice = usb, menu = menu,
            onSelect = { vm.selectSource(it) }, onAction = { vm.sourceAction(it) }, onWatch = vm::watchSourceMenu,
            onDismiss = { sheet = Sheet.None },
        )
        Sheet.Memories -> MemoriesSheet(
            memories = data.memories, currentFrequency = stateHolder.value.vfoFrequency,
            onSave = vm::saveMemory, onRecall = vm::recall, onRename = { m, n -> vm.renameMemory(m, n) },
            onDelete = { m ->
                vm.deleteMemory(m)
                scope.launch {
                    val r = snackbar.showSnackbar("« ${m.name} » supprimée", actionLabel = "Annuler", withDismissAction = true)
                    if (r == SnackbarResult.ActionPerformed) vm.restoreMemory(m)
                }
            },
            onDismiss = { sheet = Sheet.None },
        )
        Sheet.None -> Unit
    }
}

@Composable
private fun TopRow(
    source: String,
    usb: Boolean,
    playing: Boolean,
    playEnabled: Boolean,
    onPlay: () -> Unit,
    onSource: () -> Unit,
    onMemories: () -> Unit,
    onSettings: () -> Unit,
    onFt8: () -> Unit,
    ft8Active: Boolean,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) {
            AssistChip(
                onClick = onSource,
                label = { Text(source.ifBlank { "Choisir une source" }, maxLines = 1) },
                leadingIcon = { Icon(if (usb) Icons.Default.Usb else Icons.Default.Router, null, Modifier.size(AssistChipDefaults.IconSize)) },
            )
        }
        IconButton(onClick = onFt8) {
            // Badge while the decoder runs, so it isn't forgotten (it costs CPU during reception)
            BadgedBox(badge = { if (ft8Active) Badge(containerColor = MaterialTheme.colorScheme.primary) }) {
                Icon(Icons.Default.Forum, if (ft8Active) "FT8 / FT4 (décodeur actif)" else "FT8 / FT4")
            }
        }
        IconButton(onClick = onMemories) { Icon(Icons.Default.Bookmarks, "Fréquences") }
        IconButton(onClick = onSettings) { Icon(Icons.Default.Settings, "Réglages") }
        FilledIconButton(
            onClick = onPlay,
            enabled = playEnabled,
            modifier = Modifier.size(52.dp),
            colors = if (playing) IconButtonDefaults.filledIconButtonColors(
                containerColor = MaterialTheme.colorScheme.secondaryContainer,
                contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
            ) else IconButtonDefaults.filledIconButtonColors(),
        ) {
            AnimatedContent(playing, label = "play") { p ->
                Icon(
                    if (p) Icons.Default.Stop else Icons.Default.PlayArrow,
                    if (p) "Arrêter" else "Démarrer",
                    Modifier.size(28.dp),
                )
            }
        }
    }
}

/** Values the control panel needs; equal values => the panel is skipped on recomposition. */
private data class ControlValues(
    val mode: Mode?,
    val bandwidth: Double,
    val squelchEnabled: Boolean,
    val squelchLevel: Float,
    val volume: Float,
)

@Composable
private fun Header(stateHolder: State<RadioState>, onTune: (Double) -> Unit) {
    val state = stateHolder.value
    // Own layer: frequency updates don't re-record the rest of the panel
    Column(Modifier.fillMaxWidth().padding(top = 2.dp).graphicsLayer()) {
        FrequencyDisplay(state.vfoFrequency, onTune, Modifier.fillMaxWidth())
        Text(
            buildString {
                append(state.mode?.label ?: "—")
                if (state.vfoBandwidth > 0) append(" · ${formatHz(state.vfoBandwidth)}")
                if (state.sampleRate > 0) append(" · span ${formatHz(state.sampleRate)}")
            },
            style = ReadoutStyle,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
