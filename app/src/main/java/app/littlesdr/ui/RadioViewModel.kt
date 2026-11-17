package app.littlesdr.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.littlesdr.LittleSdrApp
import app.littlesdr.data.AppData
import app.littlesdr.data.Appearance
import app.littlesdr.data.DisplaySettings
import app.littlesdr.data.Memory
import app.littlesdr.engine.AdvancedOptions
import app.littlesdr.engine.Engine
import app.littlesdr.engine.NativeBridge
import app.littlesdr.engine.Mode
import app.littlesdr.engine.RadioState
import app.littlesdr.engine.SmAction
import app.littlesdr.engine.SmWidget
import app.littlesdr.service.RadioService
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** Visible part of the spectrum, relative to the hardware center frequency (Hz). */
data class ViewState(val bandwidth: Double = 0.0, val offset: Double = 0.0)

class RadioViewModel(app: Application) : AndroidViewModel(app) {
    private val littleApp = app as LittleSdrApp
    private val store = littleApp.store

    val ready = Engine.ready
    val state: StateFlow<RadioState> = Engine.state
    val signalDb: StateFlow<Float> = Engine.signalDb
    val sources = Engine.sources
    val selectedSource = Engine.selectedSource
    val usbDevice = littleApp.usb.connected

    val data: StateFlow<AppData> = store.data.stateIn(viewModelScope, SharingStarted.Eagerly, AppData())

    private val _view = MutableStateFlow(ViewState())
    val view: StateFlow<ViewState> = _view.asStateFlow()

    /** Tuning step per mode (Hz), defaults from [Mode.defaultStep]. */
    private val _steps = MutableStateFlow<Map<Mode, Double>>(emptyMap())
    val steps: StateFlow<Map<Mode, Double>> = _steps.asStateFlow()

    private val _sourceMenu = MutableStateFlow<List<SmWidget>>(emptyList())
    val sourceMenu: StateFlow<List<SmWidget>> = _sourceMenu.asStateFlow()

    /** Gain sliders and AGC switches of the current receiver, shown on the main panel. */
    val gainControls: StateFlow<List<SmWidget>> = sourceMenu
        .map { menu -> menu.filter(::isGainControl) }
        .distinctUntilChanged()
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    private fun isGainControl(w: SmWidget): Boolean {
        val gainWords = listOf("gain", "lna", "vga", "mixer", "attenuat")
        return when (w) {
            is SmWidget.SliderInt -> (w.title ?: w.label).lowercase().let { t -> gainWords.any { it in t } }
            is SmWidget.SliderFloat -> (w.title ?: w.label).lowercase().let { t -> gainWords.any { it in t } }
            is SmWidget.Checkbox -> "agc" in w.label.lowercase()
            else -> false
        }
    }
    private var menuPoller: Job? = null

    // Renderer colors (fill, line, background), set from the Compose theme
    private var displayColors = Triple(0x598CB4FF.toInt(), 0xFFCAD6FF.toInt(), 0xFF0C0E13.toInt())

    private val _advanced = MutableStateFlow(AdvancedOptions())
    val advanced: StateFlow<AdvancedOptions> = _advanced.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    init {
        // New sample rate (source change / samplerate change): show the full span again
        viewModelScope.launch {
            Engine.state.map { it.sampleRate }.distinctUntilChanged().collect { sr ->
                if (sr > 0) setView(sr, 0.0)
            }
        }
        // Keep the VFO inside the visible span when it is tuned elsewhere
        viewModelScope.launch {
            Engine.state.map { it.vfoOffset }.distinctUntilChanged().collect { followVfo() }
        }
        // Automatic FM bandwidth: measure, smooth, apply only meaningful changes
        viewModelScope.launch {
            var smoothed = 0.0
            while (isActive) {
                delay(250)
                val s = state.value
                val m = s.mode
                if (!data.value.radio.autoBandwidth || !s.playing || (m != Mode.NFM && m != Mode.WFM)) {
                    smoothed = 0.0
                    continue
                }
                val r = m.bandwidthRange
                val est = Engine.call { NativeBridge.estimateBandwidth(r.start.toDouble(), r.endInclusive.toDouble()) }.toDouble()
                if (est <= 0) continue
                smoothed = if (smoothed == 0.0) est else smoothed * 0.7 + est * 0.3
                val step = m.bandwidthStep.toDouble()
                val target = snap(smoothed, step).coerceIn(r.start.toDouble(), r.endInclusive.toDouble())
                if (kotlin.math.abs(target - s.vfoBandwidth) >= maxOf(step, s.vfoBandwidth * 0.06)) {
                    Engine.setBandwidth(target.toFloat())
                }
            }
        }
        // Source settings feed the gain block of the main panel: keep them fresh
        viewModelScope.launch {
            while (isActive) {
                if (Engine.ready.value) _sourceMenu.value = Engine.sourceMenu()
                delay(2000)
            }
        }
        viewModelScope.launch {
            Engine.state.map { it.playing }.distinctUntilChanged().collect {
                if (Engine.ready.value) _sourceMenu.value = Engine.sourceMenu()
            }
        }
        // Demodulator options depend on the mode: refresh them when it changes
        viewModelScope.launch {
            Engine.state.map { it.mode }.distinctUntilChanged().collect { refreshAdvanced() }
        }
        // Push display preferences to the renderer
        viewModelScope.launch {
            data.map { it.display }.distinctUntilChanged().collect { applyDisplay(it) }
        }
    }

    fun stepFor(mode: Mode?): Double = mode?.let { _steps.value[it] ?: it.defaultStep } ?: 1000.0
    fun setStep(mode: Mode, step: Double) { _steps.value = _steps.value + (mode to step) }

    // ------------------------------------------------------------------ playback
    fun togglePlay() {
        viewModelScope.launch {
            if (state.value.playing) {
                Engine.stop()
            } else {
                if (Engine.start()) {
                    RadioService.start(getApplication())
                } else {
                    _error.value = "Impossible de démarrer « ${selectedSource.value} ». Vérifie les réglages de la source."
                }
            }
        }
    }

    fun clearError() { _error.value = null }

    // ------------------------------------------------------------------ tuning
    fun tune(hz: Double) = Engine.tune(hz.coerceIn(1.0, 7.25e9))
    fun tuneBy(deltaHz: Double) = tune(state.value.vfoFrequency + deltaHz)
    fun step(direction: Int) {
        val s = stepFor(state.value.mode)
        tune(snap(state.value.vfoFrequency, s) + direction * s)
    }

    fun setMode(m: Mode) = Engine.setMode(m)
    fun setBandwidth(hz: Float) = Engine.setBandwidth(hz)
    fun setSquelch(enabled: Boolean, level: Float) = Engine.setSquelch(enabled, level)
    fun setVolume(v: Float) = Engine.setVolume(v)

    fun setView(bandwidth: Double, offset: Double) {
        val sr = state.value.sampleRate
        if (sr <= 0) return
        val bw = bandwidth.coerceIn(minOf(2_000.0, sr), sr)
        val maxOff = (sr - bw) / 2
        val v = ViewState(bw, offset.coerceIn(-maxOff, maxOff))
        if (v != _view.value) {
            _view.value = v
            Engine.setView(v.bandwidth, v.offset)
        }
    }

    fun resetZoom() = setView(state.value.sampleRate, 0.0)

    private fun followVfo() {
        val v = _view.value
        val s = state.value
        if (v.bandwidth <= 0) return
        val margin = v.bandwidth * 0.05
        val lo = v.offset - v.bandwidth / 2 + margin
        val hi = v.offset + v.bandwidth / 2 - margin
        // Minimal pan: the view scrolls with the VFO instead of jumping past it
        when {
            s.vfoOffset < lo -> setView(v.bandwidth, s.vfoOffset + v.bandwidth / 2 - margin)
            s.vfoOffset > hi -> setView(v.bandwidth, s.vfoOffset - v.bandwidth / 2 + margin)
        }
    }

    // ------------------------------------------------------------------ display
    fun setDisplay(s: DisplaySettings) = viewModelScope.launch { store.setDisplay(s) }
    fun setAppearance(a: Appearance) = viewModelScope.launch { store.setAppearance(a) }

    fun refreshAdvanced() = viewModelScope.launch { if (Engine.ready.value) _advanced.value = Engine.options() }
    fun setOption(key: String, value: Double) = viewModelScope.launch { _advanced.value = Engine.setOption(key, value) }
    fun setOption(key: String, value: Boolean) = setOption(key, if (value) 1.0 else 0.0)

    /** Audio outputs for the picker: id 0 = automatic (system route). */
    fun audioOutputs(): List<Pair<Int, String>> {
        val am = getApplication<Application>().getSystemService(android.media.AudioManager::class.java)
        val devices = am.getDevices(android.media.AudioManager.GET_DEVICES_OUTPUTS)
            .filter { it.isSink }
            .map { d ->
                val type = when (d.type) {
                    android.media.AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "Haut-parleur"
                    android.media.AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "Écouteur interne"
                    android.media.AudioDeviceInfo.TYPE_WIRED_HEADSET, android.media.AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "Casque filaire"
                    android.media.AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, android.media.AudioDeviceInfo.TYPE_BLE_HEADSET,
                    android.media.AudioDeviceInfo.TYPE_BLE_SPEAKER -> "Bluetooth"
                    android.media.AudioDeviceInfo.TYPE_USB_HEADSET, android.media.AudioDeviceInfo.TYPE_USB_DEVICE -> "USB"
                    android.media.AudioDeviceInfo.TYPE_HDMI -> "HDMI"
                    else -> null
                } ?: return@map null
                val name = d.productName?.toString()?.takeIf { it.isNotBlank() && type != "Haut-parleur" && type != "Écouteur interne" }
                d.id to (if (name != null) "$type · $name" else type)
            }
            .filterNotNull()
            .distinctBy { it.second }
        return listOf(0 to "Automatique (système)") + devices
    }
    fun setAutoBandwidth(on: Boolean) = viewModelScope.launch { store.setRadio(data.value.radio.copy(autoBandwidth = on)) }

    fun setDisplayColors(fill: Int, line: Int, background: Int) {
        displayColors = Triple(fill, line, background)
        applyDisplay(data.value.display)
    }

    private fun applyDisplay(d: DisplaySettings) {
        val density = getApplication<Application>().resources.displayMetrics.density
        app.littlesdr.engine.NativeBridge.setDisplay(
            d.dbMin, d.dbMax, d.spectrumFraction,
            displayColors.first, displayColors.second, displayColors.third, 1.5f * density, d.palette,
        )
    }

    // ------------------------------------------------------------------ sources
    fun selectSource(name: String) = viewModelScope.launch {
        Engine.selectSource(name)
        _sourceMenu.value = Engine.sourceMenu()
    }

    fun sourceAction(a: SmAction) = viewModelScope.launch { _sourceMenu.value = Engine.sourceMenu(a) }

    /** Refresh the settings menu while the sheet is open (status texts, device lists). */
    fun watchSourceMenu(open: Boolean) {
        menuPoller?.cancel()
        if (!open) return
        menuPoller = viewModelScope.launch {
            while (isActive) {
                if (Engine.ready.value) _sourceMenu.value = Engine.sourceMenu()
                delay(1000)
            }
        }
    }

    // ------------------------------------------------------------------ memories
    fun saveMemory(name: String) {
        val s = state.value
        viewModelScope.launch {
            store.addMemory(
                Memory(
                    name = name.ifBlank { formatFrequency(s.vfoFrequency) },
                    frequency = s.vfoFrequency,
                    mode = s.mode?.ordinal ?: Mode.NFM.ordinal,
                    bandwidth = s.vfoBandwidth.toFloat(),
                )
            )
        }
    }

    fun recall(m: Memory) {
        Mode.of(m.mode)?.let { Engine.setMode(it) }
        if (m.bandwidth > 0) Engine.setBandwidth(m.bandwidth)
        tune(m.frequency)
    }

    fun renameMemory(m: Memory, name: String) = viewModelScope.launch { store.updateMemory(m.copy(name = name)) }
    fun deleteMemory(m: Memory) = viewModelScope.launch { store.deleteMemory(m.id) }
    fun restoreMemory(m: Memory) = viewModelScope.launch { store.restoreMemory(m) }
}
