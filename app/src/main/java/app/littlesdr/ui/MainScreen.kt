package app.littlesdr.ui

import app.littlesdr.ui.theme.pillBorder
import android.content.res.Configuration
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.filled.Close
import androidx.compose.ui.platform.LocalConfiguration

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
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Router
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material3.AssistChip
import androidx.compose.material3.FilterChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.engine.RadioState
import app.littlesdr.ui.components.AudioTab
import app.littlesdr.ui.components.BandsTab
import app.littlesdr.ui.components.ControlTab
import app.littlesdr.ui.components.ControlTabBar
import app.littlesdr.ui.components.RadioTab
import app.littlesdr.ui.components.FrequencyDisplay
import app.littlesdr.ui.components.MemoriesPanel
import app.littlesdr.ui.components.MorseTab
import app.littlesdr.ui.components.SstvTab
import app.littlesdr.ui.components.RecordTab
import app.littlesdr.ui.components.KiwiServerButton
import app.littlesdr.ui.components.SourcePanel
import app.littlesdr.ui.components.SpectrumPanel
import app.littlesdr.ui.components.TuningJoystick
import app.littlesdr.ui.components.AppSpace
import app.littlesdr.ui.components.DigitalNowPlaying
import app.littlesdr.ui.components.DigitalHeader
import app.littlesdr.ui.components.DabTuner
import app.littlesdr.ui.components.DrmTuner
import app.littlesdr.ui.components.AdsbMap
import app.littlesdr.ui.components.DrmRadioScreen
import app.littlesdr.ui.components.IqFilePicker
import app.littlesdr.ui.components.AdsbPanel
import app.littlesdr.ui.components.LrptImagePanel
import app.littlesdr.ui.components.LrptPanel
import app.littlesdr.ui.components.SpaceSwitcher
import app.littlesdr.ui.components.CarRadioScreen
import app.littlesdr.ui.components.SpaceTitle
import app.littlesdr.ui.theme.ReadoutStyle
import kotlinx.coroutines.launch


@OptIn(ExperimentalMaterial3Api::class)
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
    val receiverControls by vm.receiverControls.collectAsStateWithLifecycle()
    val cw by vm.cw.collectAsStateWithLifecycle()
    val sstv by vm.sstv.collectAsStateWithLifecycle()
    val rec by vm.rec.collectAsStateWithLifecycle()
    val recordings by vm.recordings.collectAsStateWithLifecycle()
    val sstvImage by vm.sstvImage.collectAsStateWithLifecycle()
    val sstvGallery by vm.sstvGallery.collectAsStateWithLifecycle()
    val kiwiServers by vm.kiwiServers.collectAsStateWithLifecycle()
    val kiwiLoading by vm.kiwiLoading.collectAsStateWithLifecycle()
    val rds by vm.rds.collectAsStateWithLifecycle()
    val eibi by vm.eibi.collectAsStateWithLifecycle()
    val advanced by vm.advanced.collectAsStateWithLifecycle()
    val ft8State by vm.ft8.collectAsStateWithLifecycle()
    val ft8Active = ft8State.enabled
    val error by vm.error.collectAsStateWithLifecycle()
    vm.steps.collectAsStateWithLifecycle()
    val step = vm.stepFor(mode)

    var tab by rememberSaveable { mutableStateOf(ControlTab.Radio) }
    val space by vm.space.collectAsStateWithLifecycle()
    val digital by vm.digital.collectAsStateWithLifecycle()
    val slide by vm.slide.collectAsStateWithLifecycle()
    val lrpt by vm.lrpt.collectAsStateWithLifecycle()
    val lrptImage by vm.lrptImage.collectAsStateWithLifecycle()
    val adsb by vm.adsb.collectAsStateWithLifecycle()
    val adsbSelected by vm.adsbSelected.collectAsStateWithLifecycle()
    val adsbRoutes by vm.adsbRoutes.collectAsStateWithLifecycle()
    val airframe by vm.airframe.collectAsStateWithLifecycle()
    val airframePhoto by vm.airframePhoto.collectAsStateWithLifecycle()
    val dabFound by vm.dabFound.collectAsStateWithLifecycle()
    val dabScan by vm.dabScan.collectAsStateWithLifecycle()
    val kiwiRefused by vm.kiwiRefused.collectAsStateWithLifecycle()
    val dabAf by vm.dabAf.collectAsStateWithLifecycle()
    val dabCatalog by vm.catalog.collectAsStateWithLifecycle()
    val afSearching by vm.afSearching.collectAsStateWithLifecycle()
    // Outside the SDR space, the receiver chip opens the source settings in place
    var showSource by rememberSaveable { mutableStateOf(false) }
    // Digital space: audio settings open from the player card
    var audioSheet by rememberSaveable { mutableStateOf(false) }

    val sourcePanel = @Composable {
        SourcePanel(
            sources = sources, selected = selectedSource, playing = playing, usbDevice = usb, menu = menu,
            onSelect = { vm.selectSource(it) }, onAction = { vm.sourceAction(it) }, onWatch = vm::watchSourceMenu,
            extra = {
                if (selectedSource == "Fichier IQ") {
                    val iqFiles by vm.iqFiles.collectAsStateWithLifecycle()
                    val iqImport by vm.iqImport.collectAsStateWithLifecycle()
                    val current = menu.filterIsInstance<app.littlesdr.engine.SmWidget.InputText>().firstOrNull { it.id == "file_path" }?.value ?: ""
                    IqFilePicker(
                        files = iqFiles, current = current, importing = iqImport, enabled = !playing,
                        onRefresh = vm::refreshIqFiles, onImport = vm::importIq, onPick = { vm.pickIqFile(it) },
                    )
                }
                if (selectedSource == "KiwiSDR") {
                    KiwiServerButton(
                        servers = kiwiServers, loading = kiwiLoading, position = vm.operatorPosition(), refused = kiwiRefused,
                        onLoad = { vm.loadKiwiServers(it) },
                        onPick = { hp, name -> vm.setKiwiSite(hp, name) },
                    )
                }
            },
        )
    }
    val audioControls = @Composable {
        AudioTab(
            volume = controls.volume,
            onVolume = vm::setVolume,
            outputs = remember { vm.audioOutputs() },
            device = advanced.audio.device,
            latency = advanced.audio.latency,
            onDevice = { vm.setOption("audio.device", it.toDouble()) },
            onLatency = { vm.setOption("audio.latency", it.toDouble()) },
            stereo = advanced.demod.stereo.takeIf { advanced.demod.stereoAllowed },
            onStereo = { vm.setOption("demod.stereo", it) },
        )
    }

    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    // Renderer colors follow the Material theme (dynamic color included)
    val cs = MaterialTheme.colorScheme
    LaunchedEffect(cs.primary, cs.surfaceContainerLowest) {
        vm.setDisplayColors(cs.primary.copy(alpha = 0.35f).toArgb(), cs.primary.toArgb(), cs.surfaceContainerLowest.toArgb())
    }
    LaunchedEffect(error) {
        error?.let { snackbar.showSnackbar(it, withDismissAction = true, duration = SnackbarDuration.Long); vm.clearError() }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        containerColor = MaterialTheme.colorScheme.surface,
    ) { _ ->
        Box(Modifier.fillMaxSize()) {
            // Margins keep everything clear of rounded screen corners, cutouts and the nav bar.
            val corners = rememberScreenCorners()
            val m = corners.margin
            // Every space upright or sideways, as the phone is held (no rotation forced when
            // switching: a space laid out for the other orientation, then turned, broke the
            // spectrum's surface)
            if (space == AppSpace.DAB) {
                CarRadioScreen(
                    d = digital, slide = slide, frequency = stateHolder.value.vfoFrequency,
                    playing = playing, playEnabled = ready,
                    found = dabFound, scan = dabScan, af = dabAf,
                    source = selectedSource, usb = usb != null, space = space, cornerRadius = corners.innerRadius,
                    catalog = dabCatalog, onCatalog = vm::playCatalog, afSearching = afSearching,
                    onPlay = vm::togglePlay, onStep = vm::dabStep,
                    onService = vm::dabSelect, onTune = vm::tuneDab, onScan = vm::scanDab, onAf = vm::setDabAf,
                    onSource = { showSource = true }, onSettings = onOpenSettings,
                    onSpace = { showSource = false; vm.setSpace(it) },
                    modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing.union(WindowInsets(m, m, m, m))),
                )
            } else if (space == AppSpace.DRM) {
                // DRM: a shortwave receiver of its own (station, schedule, received stations)
                val drmHeard by vm.drmHeard.collectAsStateWithLifecycle()
                val drmSchedule by vm.drmSchedule.collectAsStateWithLifecycle()
                DrmRadioScreen(
                    d = digital, slide = slide, frequency = stateHolder.value.vfoFrequency,
                    playing = playing, playEnabled = ready,
                    source = selectedSource, usb = usb != null, space = space, cornerRadius = corners.innerRadius,
                    eibi = eibi, heard = drmHeard, schedule = drmSchedule, controls = receiverControls,
                    spectrum = { mod ->
                        SpectrumPanel(
                            state = stateHolder, view = viewHolder, display = data.display, step = step,
                            onTune = vm::tune, onView = vm::setView, onResetZoom = vm::resetZoom,
                            cornerRadius = 16.dp,
                            maskColor = MaterialTheme.colorScheme.surfaceContainerLow,
                            rds = rds,
                            eibi = emptyList(),
                            modifier = mod,
                        )
                    },
                    onPlay = vm::togglePlay, onStep = vm::drmStep, onTune = vm::tune, onForget = vm::forgetDrm,
                    onControl = { vm.sourceAction(it) },
                    onSource = { showSource = true }, onSettings = onOpenSettings,
                    onSpace = { showSource = false; vm.setSpace(it) },
                    modifier = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing.union(WindowInsets(m, m, m, m))),
                )
            } else {
                // Landscape: the view (spectrum, map, picture) on the left, the control panel on
                // the right. Portrait: the view on top, the panel under it.
                val portrait = LocalConfiguration.current.orientation == Configuration.ORIENTATION_PORTRAIT
                val r = corners.innerRadius
                // Head-unit player card, shared by DAB+ (whole height) and DRM (under the spectrum)
                val player = @Composable { m: Modifier ->
                    DigitalNowPlaying(
                        d = digital, slide = slide, isDab = space == AppSpace.DAB, frequency = stateHolder.value.vfoFrequency,
                        playing = playing, playEnabled = ready, volume = controls.volume,
                        onPlay = vm::togglePlay, onVolume = vm::setVolume, onAudioSettings = { audioSheet = true },
                        cornerRadius = r,
                        modifier = m,
                    )
                }
                val viewPane = @Composable { mod: Modifier ->
                    Column(mod, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    // Spectrum: big in SDR; a strip above the picture (LRPT) or the player (DRM),
                    // where a DRM block seen in the band can be tapped to tune it
                    // ADS-B: the map gets the whole height, the band has nothing to look at
                    if (space != AppSpace.ADSB) SpectrumPanel(
                        state = stateHolder, view = viewHolder, display = data.display, step = step,
                        onTune = vm::tune, onView = vm::setView, onResetZoom = vm::resetZoom,
                        cornerRadius = corners.innerRadius,
                        maskColor = MaterialTheme.colorScheme.surface,
                        rds = rds,
                        eibi = eibi,
                        modifier = Modifier.weight(
                            when (space) { AppSpace.SDR -> 1f; AppSpace.DRM -> 0.34f; else -> 0.38f },
                        ).fillMaxWidth(),
                    )
                    when (space) {
                        AppSpace.SDR -> TuningJoystick(
                            // Rate joystick: deflection = tuning speed, springs back to stop
                            frequency = { stateHolder.value.vfoFrequency },
                            span = { viewHolder.value.bandwidth.takeIf { it > 0 } ?: stateHolder.value.sampleRate },
                            step = { vm.stepFor(stateHolder.value.mode) },
                            onTune = vm::tune,
                            modifier = Modifier.fillMaxWidth().height(52.dp),
                        )
                        AppSpace.LRPT -> LrptImagePanel(lrptImage, lrpt, playing, Modifier.weight(0.62f).fillMaxWidth())
                        AppSpace.DRM -> player(Modifier.weight(0.66f).fillMaxWidth())
                        AppSpace.ADSB -> AdsbMap(adsb, adsbSelected, playing, vm::selectAircraft, Modifier.weight(1f).fillMaxWidth())
                        AppSpace.DAB -> Unit
                    }
                    }
                }
                // Right "pill" cut in three: the top bar is split (tabs or title | space
                // switcher), the selected tab's controls below.
                val controlPane = @Composable { mod: Modifier ->
                    Column(mod, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(Modifier.fillMaxWidth().height(64.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Surface(
                            color = MaterialTheme.colorScheme.surfaceContainerLow,
                            border = pillBorder(),
                            shape = RoundedCornerShape(topStart = r, topEnd = 6.dp, bottomStart = 6.dp, bottomEnd = 6.dp),
                            modifier = Modifier.weight(1f).fillMaxHeight(),
                        ) {
                            if (space == AppSpace.SDR) ControlTabBar(tab, onSelect = { tab = it }, modifier = Modifier.fillMaxSize())
                            else SpaceTitle(space, Modifier.fillMaxSize())
                        }
                        SpaceSwitcher(
                            space = space,
                            onSpace = { showSource = false; vm.setSpace(it) },
                            shape = RoundedCornerShape(topStart = 6.dp, topEnd = r, bottomStart = 6.dp, bottomEnd = 6.dp),
                            modifier = Modifier.width(72.dp).fillMaxHeight(),
                        )
                    }
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceContainerLow,
                        border = pillBorder(),
                        shape = RoundedCornerShape(topStart = 6.dp, topEnd = 6.dp, bottomStart = r, bottomEnd = r),
                        modifier = Modifier.fillMaxWidth().weight(1f),
                    ) {
                        if (space == AppSpace.DAB || space == AppSpace.DRM) Column(Modifier.fillMaxSize().padding(horizontal = 14.dp, vertical = 10.dp)) {
                            // No outer scroll here: the station list owns all the height left
                            DigitalHeader(
                                source = selectedSource, usb = usb != null, sourceOpen = showSource,
                                onSource = { showSource = true }, onSettings = onOpenSettings,
                                modifier = Modifier.fillMaxWidth(),
                            )
                            Spacer(Modifier.height(10.dp))
                            if (space == AppSpace.DRM) DrmTuner(
                                frequency = stateHolder.value.vfoFrequency, d = digital, onTune = vm::tune,
                                controls = receiverControls, onControl = { vm.sourceAction(it) },
                                modifier = Modifier.weight(1f).fillMaxWidth(),
                            )
                            else DabTuner(
                                frequency = stateHolder.value.vfoFrequency, d = digital,
                                found = dabFound, scan = dabScan,
                                onTune = vm::tuneDab, onService = vm::dabSelect, onScan = vm::scanDab,
                                af = dabAf, onAf = vm::setDabAf,
                                modifier = Modifier.weight(1f).fillMaxWidth(),
                            )
                        } else Column(
                            Modifier
                                .fillMaxSize()
                                .verticalScroll(rememberScrollState())
                                .padding(horizontal = 14.dp, vertical = 8.dp),
                        ) {
                            TopRow(
                                source = selectedSource,
                                usb = usb != null,
                                playing = playing,
                                playEnabled = ready,
                                onPlay = vm::togglePlay,
                                onSource = { if (space == AppSpace.SDR) tab = ControlTab.Source else showSource = true },
                                onSettings = onOpenSettings,
                                onFt8 = onOpenFt8,
                                ft8Active = ft8Active,
                                showFt8 = space == AppSpace.SDR,
                            )
                            // The frequency where tuning happens: Radio, Bands, Record (and LRPT); the
                            // receiver, audio, memories, Morse and SSTV tabs don't need it
                            if (space != AppSpace.ADSB && (space != AppSpace.SDR || tab == ControlTab.Radio || tab == ControlTab.Bands || tab == ControlTab.Record)) {
                                Header(stateHolder, onTune = vm::tune)
                            }
                            Spacer(Modifier.height(8.dp))
                            if (space != AppSpace.SDR) {
                                when {
                                    space == AppSpace.ADSB -> AdsbPanel(adsb, adsbSelected, adsbRoutes, airframe, airframePhoto, onSelect = vm::selectAircraft, onClear = vm::clearAircraft)
                                    else -> LrptPanel(
                                        frequency = stateHolder.value.vfoFrequency, l = lrpt,
                                        onTune = vm::tune, onOqpsk = vm::setLrptOqpsk, onSave = vm::saveLrpt,
                                    )
                                }
                            } else when (tab) {
                                ControlTab.Source -> sourcePanel()
                                ControlTab.Radio -> RadioTab(
                                    mode = controls.mode,
                                    bandwidth = controls.bandwidth,
                                    autoBandwidth = data.radio.autoBandwidth,
                                    step = step,
                                    signalDb = vm.signalDb,
                                    onMode = vm::setMode,
                                    onStep = { s -> mode?.let { vm.setStep(it, s) } },
                                    onBandwidth = vm::setBandwidth,
                                    onAutoBandwidth = { vm.setAutoBandwidth(it) },
                                    gain = gainControls,
                                    onGainAction = { vm.sourceAction(it) },
                                    squelchEnabled = controls.squelchEnabled,
                                    squelchLevel = controls.squelchLevel,
                                    onSquelch = vm::setSquelch,
                                )
                                ControlTab.Audio -> audioControls()
                                ControlTab.Bands -> BandsTab(stateHolder.value.vfoFrequency, vm::applyPreset)
                                ControlTab.Morse -> MorseTab(
                                    cw = cw, mode = controls.mode,
                                    onEnable = { vm.setCwEnabled(it) }, onWatch = vm::watchCw, onTune = vm::tuneCw,
                                )
                                ControlTab.Sstv -> SstvTab(
                                    sstv = sstv, image = sstvImage, gallery = sstvGallery,
                                    onEnable = { vm.setSstvEnabled(it) },
                                    onStartManual = { vm.sstvStartManual(it) },
                                    onFinish = { vm.sstvFinish() },
                                    onRefreshGallery = { vm.refreshSstvGallery() },
                                    onDelete = { f -> vm.deleteFile(f) { vm.refreshSstvGallery() } },
                                )
                                ControlTab.Record -> RecordTab(
                                    rec = rec, recordings = recordings, playing = playing, folder = vm.storageLabel(),
                                    onWatch = vm::watchRecorder,
                                    onSet = { k, v -> vm.setRecorder(k, v) },
                                    onDelete = { f -> vm.deleteFile(f) { vm.refreshRecordings() } },
                                )
                                ControlTab.Memories -> MemoriesPanel(
                                    memories = data.memories, currentFrequency = stateHolder.value.vfoFrequency,
                                    suggestedName = rds.ps.trim(),
                                    onSave = vm::saveMemory, onRecall = vm::recall, onRename = { m, n -> vm.renameMemory(m, n) },
                                    onDelete = { m ->
                                        vm.deleteMemory(m)
                                        scope.launch {
                                            val r = snackbar.showSnackbar("« ${m.name} » supprimée", actionLabel = "Annuler", withDismissAction = true)
                                            if (r == SnackbarResult.ActionPerformed) vm.restoreMemory(m)
                                        }
                                    },
                                )
                            }
                        }
                    }
                    }
                }
                val frame = Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing.union(WindowInsets(m, m, m, m)))
                if (portrait) Column(frame, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    // SDR: the controls need more room than the spectrum
                    val top = if (space == AppSpace.SDR) 0.42f else 0.5f
                    viewPane(Modifier.weight(top).fillMaxWidth())
                    controlPane(Modifier.weight(1f - top).fillMaxWidth())
                } else Row(frame, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    viewPane(Modifier.weight(1f).fillMaxHeight())
                    // 330 dp: just what the widest rows need, the rest goes to the waterfall
                    controlPane(Modifier.width(330.dp).fillMaxHeight())
                }
            }

            // Receiver settings outside the SDR space: a floating card over the page, whatever
            // the page (car radio, DRM, LRPT, ADS-B), closed by ✕ or a tap outside
            if (showSource && space != AppSpace.SDR) {
                androidx.compose.ui.window.Dialog(
                    onDismissRequest = { showSource = false },
                    properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false),
                ) {
                    val conf = LocalConfiguration.current
                    Surface(
                        shape = RoundedCornerShape(28.dp),
                        color = MaterialTheme.colorScheme.surfaceContainerHigh,
                        tonalElevation = 6.dp,
                        shadowElevation = 16.dp,
                        modifier = Modifier
                            .widthIn(max = 460.dp)
                            .fillMaxWidth(0.92f)
                            .heightIn(max = (conf.screenHeightDp * 0.85f).dp),
                    ) {
                        Column(Modifier.padding(top = 8.dp, bottom = 16.dp)) {
                            Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                                Icon(if (usb != null) Icons.Default.Usb else Icons.Default.Router, null, tint = MaterialTheme.colorScheme.primary)
                                Spacer(Modifier.width(10.dp))
                                Text("Récepteur", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                                IconButton(onClick = { showSource = false }) { Icon(Icons.Default.Close, "Fermer") }
                            }
                            Column(Modifier.padding(horizontal = 20.dp).verticalScroll(rememberScrollState())) { sourcePanel() }
                        }
                    }
                }
            }

            if (audioSheet) {
                ModalBottomSheet(onDismissRequest = { audioSheet = false }) {
                    Column(
                        Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(start = 24.dp, end = 24.dp, bottom = 24.dp),
                    ) {
                        Text("Audio", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(bottom = 8.dp))
                        audioControls()
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

}

@Composable
private fun TopRow(
    source: String,
    usb: Boolean,
    playing: Boolean,
    playEnabled: Boolean,
    onPlay: () -> Unit,
    onSource: () -> Unit,
    onSettings: () -> Unit,
    onFt8: () -> Unit,
    ft8Active: Boolean,
    showFt8: Boolean,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) {
            AssistChip(
                onClick = onSource,
                label = { Text(source.ifBlank { "Choisir une source" }, maxLines = 1) },
                leadingIcon = { Icon(if (usb) Icons.Default.Usb else Icons.Default.Router, null, Modifier.size(AssistChipDefaults.IconSize)) },
            )
        }
        // Plain "FT8" label (an icon wasn't obvious); highlighted while the decoder runs so it
        // isn't forgotten, as it costs CPU during reception
        if (showFt8) FilterChip(
            selected = ft8Active,
            onClick = onFt8,
            label = { Text("FT8") },
            modifier = Modifier.padding(horizontal = 4.dp).semantics {
                contentDescription = if (ft8Active) "FT8 / FT4, décodeur actif" else "FT8 / FT4"
            },
        )
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
