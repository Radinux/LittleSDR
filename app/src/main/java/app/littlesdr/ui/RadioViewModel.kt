package app.littlesdr.ui

import androidx.compose.ui.graphics.asImageBitmap
import app.littlesdr.engine.DAB_CHANNELS
import app.littlesdr.engine.DabService
import app.littlesdr.engine.DigitalState
import app.littlesdr.engine.LRPT_SATELLITES
import app.littlesdr.engine.LrptState
import app.littlesdr.ui.components.AppSpace
import kotlinx.coroutines.currentCoroutineContext

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import app.littlesdr.LittleSdrApp
import app.littlesdr.data.AppData
import app.littlesdr.data.EiBiEntry
import app.littlesdr.data.PhoneLocation
import app.littlesdr.data.Appearance
import app.littlesdr.data.DisplaySettings
import app.littlesdr.data.Memory
import app.littlesdr.engine.AdvancedOptions
import app.littlesdr.engine.Engine
import app.littlesdr.data.KiwiDirectory
import app.littlesdr.data.Storage
import app.littlesdr.data.KiwiServer
import app.littlesdr.engine.CwState
import app.littlesdr.engine.SstvState
import app.littlesdr.engine.RecorderState
import app.littlesdr.engine.Ft8Link
import app.littlesdr.engine.Ft8Message
import app.littlesdr.engine.Ft8State
import app.littlesdr.engine.Ft8Station
import app.littlesdr.engine.Ft8Traffic
import app.littlesdr.engine.GeoPoint
import app.littlesdr.engine.Maidenhead
import app.littlesdr.engine.PresetEntry
import app.littlesdr.engine.RdsInfo
import app.littlesdr.engine.NativeBridge
import app.littlesdr.engine.Mode
import app.littlesdr.engine.RadioState
import app.littlesdr.engine.SmAction
import app.littlesdr.engine.SmWidget
import app.littlesdr.service.RadioService
import android.graphics.Bitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
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

    /** Gain block plus the bias-tee switch: what the DRM screen needs for an active HF antenna */
    val receiverControls: StateFlow<List<SmWidget>> = sourceMenu
        .map { menu -> menu.filter { isGainControl(it) || (it is SmWidget.Checkbox && ("bias" in it.id.lowercase() || "bias" in it.label.lowercase())) } }
        .distinctUntilChanged()
        .stateIn(viewModelScope, SharingStarted.Eagerly, emptyList())

    private fun isGainControl(w: SmWidget): Boolean {
        val gainWords = listOf("gain", "lna", "vga", "mixer", "attenuat")
        return when (w) {
            // Unlabelled sliders are recognised by their id (SpyServer: "##spyserver_source_gain")
            is SmWidget.SliderInt -> "${w.title ?: w.label} ${w.id}".lowercase().let { t -> gainWords.any { it in t } }
            is SmWidget.SliderFloat -> "${w.title ?: w.label} ${w.id}".lowercase().let { t -> gainWords.any { it in t } }
            is SmWidget.Checkbox -> "agc" in w.label.lowercase()
            else -> false
        }
    }
    private var menuPoller: Job? = null

    // Renderer colors (fill, line, background), set from the Compose theme
    private var displayColors = Triple(0x598CB4FF.toInt(), 0xFFCAD6FF.toInt(), 0xFF0C0E13.toInt())

    private val _advanced = MutableStateFlow(AdvancedOptions())
    val advanced: StateFlow<AdvancedOptions> = _advanced.asStateFlow()

    /** RDS of the station being received (WFM only), empty otherwise. */
    // ---- EiBi: shortwave broadcasts on the tuned frequency right now
    private val eibiRepo = littleApp.eibi
    private val _eibi = MutableStateFlow<List<EiBiEntry>>(emptyList())
    val eibi: StateFlow<List<EiBiEntry>> = _eibi.asStateFlow()

    data class EiBiStatus(val season: String = "", val count: Int = 0, val updated: Long = 0, val updating: Boolean = false)
    private val _eibiStatus = MutableStateFlow(EiBiStatus())
    val eibiStatus: StateFlow<EiBiStatus> = _eibiStatus.asStateFlow()

    fun updateEibi(force: Boolean = true) = viewModelScope.launch {
        _eibiStatus.value = _eibiStatus.value.copy(updating = true)
        val ok = eibiRepo.ensure(force)
        _eibiStatus.value = EiBiStatus(eibiRepo.loadedSeason.uppercase(), eibiRepo.count, eibiRepo.lastUpdate(), false)
        if (force && !ok) _error.value = "Impossible de télécharger les horaires EiBi (réseau ?)"
    }

    private val _rds = MutableStateFlow(RdsInfo())
    val rds: StateFlow<RdsInfo> = _rds.asStateFlow()

    private val _ft8 = MutableStateFlow(Ft8State())
    val ft8: StateFlow<Ft8State> = _ft8.asStateFlow()
    private var ft8Poller: Job? = null

    private val _ft8Traffic = MutableStateFlow(Ft8Traffic(emptyList(), emptyList()))
    val ft8Traffic: StateFlow<Ft8Traffic> = _ft8Traffic.asStateFlow()
    private val gridCache = HashMap<String, String>()          // callsign -> last locator seen
    private val countryCache = HashMap<String, GeoPoint?>()    // callsign -> CTY country position

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    init {
        // The SSTV decoder outlives the ViewModel (engine kept by the service): pick it up again
        viewModelScope.launch {
            Engine.ready.first { it }
            val s = Engine.sstv()
            _sstv.value = s
            if (s.enabled && sstvPoller == null) sstvPoller = viewModelScope.launch { pollSstv() }
        }
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
        // EiBi: load (download if needed), then match the VFO against the schedule every second
        viewModelScope.launch {
            updateEibi(force = false).join()
            while (isActive) {
                val s = state.value
                _eibi.value = if (s.vfoFrequency < 30e6 && s.mode != Mode.WFM) {
                    eibiRepo.lookup(s.vfoFrequency, 2_500.0, java.time.ZonedDateTime.now(java.time.ZoneOffset.UTC))
                } else emptyList()
                delay(1000)
            }
        }
        // RDS: poll once a second while receiving WFM; forget it when tuning elsewhere
        viewModelScope.launch {
            var lastFreq = 0.0
            while (isActive) {
                val s = state.value
                if (kotlin.math.abs(s.vfoFrequency - lastFreq) > 50_000) {
                    _rds.value = RdsInfo()
                    lastFreq = s.vfoFrequency
                }
                if (Engine.ready.value && s.playing && s.mode == Mode.WFM) _rds.value = Engine.rds()
                else if (_rds.value.hasData) _rds.value = RdsInfo()
                delay(1000)
            }
        }
        // Automatic locator: refresh from the phone's position once the engine is up
        viewModelScope.launch {
            Engine.ready.first { it }
            if (store.data.first().operator.autoLocator) updateLocatorFromPosition()
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

    // ------------------------------------------------------------------ spaces (SDR / DAB+ DRM / LRPT)
    private val spacePrefs = app.getSharedPreferences("spaces", android.content.Context.MODE_PRIVATE)
    private val _space = MutableStateFlow(AppSpace.entries.getOrNull(spacePrefs.getInt("space", 0)) ?: AppSpace.SDR)
    val space: StateFlow<AppSpace> = _space.asStateFlow()

    private fun lastFreq(m: Mode, default: Double): Double =
        Double.fromBits(spacePrefs.getLong("freq_${m.name}", default.toRawBits()))

    private fun rememberCurrent() {
        val s = state.value
        val m = s.mode ?: return
        spacePrefs.edit().putLong("freq_${m.name}", s.vfoFrequency.toRawBits()).putInt("mode_${_space.value.name}", m.ordinal).apply()
    }

    private fun defaultFreq(m: Mode) = when (m) {
        Mode.DAB -> DAB_CHANNELS.first { it.name == "11D" }.hz
        Mode.DRM -> 13_790_000.0
        Mode.LRPT -> LRPT_SATELLITES.first().hz
        else -> 100_000_000.0
    }

    /** Switches the interface; each space comes back to its own mode and frequency */
    fun setSpace(target: AppSpace) {
        if (target == _space.value) return
        rememberCurrent()
        _space.value = target
        spacePrefs.edit().putInt("space", target.ordinal).apply()
        val fallback = when (target) {
            AppSpace.SDR -> Mode.WFM
            AppSpace.DAB -> Mode.DAB
            AppSpace.DRM -> Mode.DRM
            AppSpace.LRPT -> Mode.LRPT
        }
        val saved = Mode.of(spacePrefs.getInt("mode_${target.name}", -1))
        val mode = saved?.takeIf { target.accepts(it) } ?: fallback
        switchTo(mode)
    }

    private fun switchTo(mode: Mode) {
        _digital.value = DigitalState()
        _slide.value = null
        Engine.setMode(mode)
        tune(lastFreq(mode, defaultFreq(mode)))
    }

    private val _digital = MutableStateFlow(DigitalState())
    val digital: StateFlow<DigitalState> = _digital.asStateFlow()
    private val _slide = MutableStateFlow<androidx.compose.ui.graphics.ImageBitmap?>(null)
    val slide: StateFlow<androidx.compose.ui.graphics.ImageBitmap?> = _slide.asStateFlow()
    private val _lrpt = MutableStateFlow(LrptState())
    val lrpt: StateFlow<LrptState> = _lrpt.asStateFlow()
    private val _lrptImage = MutableStateFlow<androidx.compose.ui.graphics.ImageBitmap?>(null)
    val lrptImage: StateFlow<androidx.compose.ui.graphics.ImageBitmap?> = _lrptImage.asStateFlow()

    fun dabSelect(s: DabService) {
        _slide.value = null
        Engine.dabSelect(s.sid)
    }

    fun setLrptOqpsk(on: Boolean) = Engine.lrptSet("oqpsk", if (on) 1.0 else 0.0)

    fun saveLrpt() {
        viewModelScope.launch {
            val files = Engine.lrptSave()
            _error.value = if (files.isEmpty()) "Pas encore d'image à enregistrer" else "${files.size} image(s) enregistrée(s)"
        }
    }

    /** DAB channels where an ensemble was found by the last scan: channel -> ensemble name */
    private val _dabFound = MutableStateFlow(
        spacePrefs.getString("dab_found", "")!!.split(';').mapNotNull { e ->
            e.split('=', limit = 2).takeIf { it.size == 2 }?.let { it[0] to it[1] }
        }.toMap()
    )
    val dabFound: StateFlow<Map<String, String>> = _dabFound.asStateFlow()

    /** Scan progress (channel index, total), null when idle */
    private val _dabScan = MutableStateFlow<Pair<Int, Int>?>(null)
    val dabScan: StateFlow<Pair<Int, Int>?> = _dabScan.asStateFlow()
    private var scanJob: Job? = null

    /** Visits every band III channel and keeps those carrying an ensemble */
    fun scanDab() {
        if (scanJob?.isActive == true) {
            scanJob?.cancel()
            return
        }
        scanJob = viewModelScope.launch {
            if (!state.value.playing && !Engine.start()) {
                _error.value = "Impossible de démarrer la source pour la recherche"
                return@launch
            }
            val found = linkedMapOf<String, String>()
            try {
                for ((i, ch) in DAB_CHANNELS.withIndex()) {
                    _dabScan.value = i to DAB_CHANNELS.size
                    tune(ch.hz)
                    delay(1300)
                    var d = Engine.digital()
                    if (d.synced) {
                        // Synced: the ensemble name follows within a few seconds
                        repeat(10) {
                            if (d.ensemble.isNotBlank()) return@repeat
                            delay(400)
                            d = Engine.digital()
                        }
                        if (d.ensemble.isNotBlank()) found[ch.name] = d.ensemble
                    }
                }
            } finally {
                _dabScan.value = null
                if (found.isNotEmpty()) {
                    _dabFound.value = found
                    spacePrefs.edit().putString("dab_found", found.entries.joinToString(";") { "${it.key}=${it.value}" }).apply()
                    DAB_CHANNELS.firstOrNull { it.name == found.keys.first() }?.let { tune(it.hz) }
                }
            }
            if (found.isEmpty()) _error.value = "Aucun multiplex DAB trouvé"
        }
    }

    // After the fields above: init blocks run in declaration order
    init {
        viewModelScope.launch { pollSpaces() }
        // The engine restores its last mode: bring the interface to the space that owns it
        // (e.g. DRM saved from the old shared DAB+ / DRM space)
        viewModelScope.launch {
            val mode = state.first { it.mode != null }.mode ?: return@launch
            if (!_space.value.accepts(mode)) {
                AppSpace.entries.firstOrNull { it.accepts(mode) }?.let {
                    _space.value = it
                    spacePrefs.edit().putInt("space", it.ordinal).apply()
                }
            }
        }
    }

    private suspend fun pollSpaces() {
        var lastSlide = 0L
        var lastPackets = -1L
        var lastImage = 0L
        while (currentCoroutineContext().isActive) {
            val sp = _space.value
            if (Engine.ready.value && state.value.playing) {
                when (sp) {
                    AppSpace.DAB, AppSpace.DRM -> {
                        val d = Engine.digital()
                        _digital.value = d
                        if (d.slide != lastSlide) {
                            lastSlide = d.slide
                            _slide.value = if (d.slide == 0L) null else Engine.dabSlide().let { b ->
                                kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {
                                    android.graphics.BitmapFactory.decodeByteArray(b, 0, b.size)?.asImageBitmap()
                                }
                            }
                        }
                    }
                    AppSpace.LRPT -> {
                        val l = Engine.lrpt()
                        _lrpt.value = l
                        val now = System.currentTimeMillis()
                        if (l.packets != lastPackets && now - lastImage > 3_000) {
                            lastPackets = l.packets
                            lastImage = now
                            val px = Engine.lrptImage()
                            _lrptImage.value = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {
                                val w = px.getOrElse(0) { 0 }
                                val h = px.getOrElse(1) { 0 }
                                if (w <= 0 || h <= 0) null
                                else android.graphics.Bitmap.createBitmap(px, 2, w, w, h, android.graphics.Bitmap.Config.ARGB_8888).asImageBitmap()
                            }
                        }
                    }
                    AppSpace.SDR -> {}
                }
            }
            delay(if (sp == AppSpace.SDR) 1000 else 700)
        }
    }

    fun stepFor(mode: Mode?): Double = mode?.let { _steps.value[it] ?: it.defaultStep } ?: 1000.0
    fun setStep(mode: Mode, step: Double) { _steps.value = _steps.value + (mode to step) }

    // ------------------------------------------------------------------ playback
    /** Set when the user stops: any other end of reception is the source giving up */
    private var userStopped = false

    fun togglePlay() {
        viewModelScope.launch {
            if (state.value.playing) {
                userStopped = true
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
        var off = offset.coerceIn(-maxOff, maxOff)
        // Keep the VFO on screen whatever the zoom/pan: shift the view just enough (5 % margin)
        val vfo = state.value.vfoOffset
        if (vfo >= -sr / 2 && vfo <= sr / 2) {
            val margin = bw * 0.05
            val lo = off - bw / 2 + margin
            val hi = off + bw / 2 - margin
            if (vfo < lo) off -= lo - vfo
            if (vfo > hi) off += vfo - hi
            off = off.coerceIn(-maxOff, maxOff)
        }
        val v = ViewState(bw, off)
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

    private val _locating = MutableStateFlow(false)
    val locating: StateFlow<Boolean> = _locating.asStateFlow()

    fun setAutoLocator(on: Boolean) = viewModelScope.launch {
        store.setOperator(data.value.operator.copy(autoLocator = on))
        if (on) updateLocatorFromPosition()
    }

    /** Sets the operator locator from the phone's position (if allowed and available). */
    fun updateLocatorFromPosition() = viewModelScope.launch {
        val ctx = getApplication<Application>()
        if (!PhoneLocation.hasPermission(ctx)) return@launch
        _locating.value = true
        val loc = PhoneLocation.current(ctx)
        _locating.value = false
        if (loc == null) {
            _error.value = "Position indisponible : active la localisation du téléphone."
            return@launch
        }
        setOptionString("operator.location", Maidenhead.fromLatLon(loc.latitude, loc.longitude))
    }

    fun setOptionString(key: String, value: String) = viewModelScope.launch { _advanced.value = Engine.setOptionString(key, value) }

    // ------------------------------------------------------------------ FT8 / FT4
    /** Poll decodes while the FT8 screen is visible. */
    fun watchFt8(visible: Boolean) {
        ft8Poller?.cancel()
        if (!visible) return
        ft8Poller = viewModelScope.launch {
            while (isActive) {
                if (Engine.ready.value) {
                    _ft8.value = Engine.ft8()
                    _ft8Traffic.value = buildTraffic(_ft8.value)
                }
                delay(1500)
            }
        }
    }

    /** Stations (with a position) and recent transmissions, for the FT8 map. */
    private suspend fun buildTraffic(s: Ft8State): Ft8Traffic {
        data class Seen(var time: Long, var cq: Boolean, var snr: Double, var heard: Boolean)
        val seen = LinkedHashMap<String, Seen>()
        val links = ArrayList<Ft8Link>()
        val newest = s.results.maxOfOrNull { it.time } ?: 0L
        for (r in s.results) { // newest first
            val m = Ft8Message.parse(r.message) ?: continue
            m.grid?.let { gridCache.putIfAbsent(m.sender, it) }
            seen.getOrPut(m.sender) { Seen(r.time, m.recipient == null, r.snr, true) }.heard = true
            m.recipient?.let { to ->
                seen.getOrPut(to) { Seen(r.time, false, Double.NaN, false) }
                if (newest - r.time <= 120_000) links += Ft8Link(m.sender, to, r.time)
            }
        }
        // Country positions for stations without a locator (one native call for all unknowns)
        val unknown = seen.keys.filter { it !in gridCache && it !in countryCache }
        if (unknown.isNotEmpty()) {
            val ll = Engine.call { NativeBridge.ctyLatLon(unknown.toTypedArray()) }
            unknown.forEachIndexed { i, c ->
                val lat = ll[2 * i]
                val lon = ll[2 * i + 1]
                countryCache[c] = if (lat.isNaN()) null else GeoPoint(lat, lon)
            }
        }
        val stations = seen.mapNotNull { (call, v) ->
            val grid = gridCache[call]?.let { Maidenhead.toLatLon(it) }
            val pos = grid ?: countryCache[call] ?: return@mapNotNull null
            Ft8Station(call, pos, grid != null, v.time, v.cq, v.snr, v.heard)
        }
        return Ft8Traffic(stations, links)
    }

    fun ft8Set(key: String, value: Boolean) = viewModelScope.launch { _ft8.value = Engine.ft8Set(key, if (value) 1.0 else 0.0) }
    fun ft8Clear() = viewModelScope.launch { _ft8.value = Engine.ft8Set("clear", 1.0) }

    /**
     * Tune the receiver on an FT8/FT4 channel: USB with the VFO on the dial frequency (the
     * engine's USB passband starts at the VFO), so you can also listen.
     */
    fun tuneDigital(dialHz: Long) {
        Engine.setMode(Mode.USB)
        tune(dialHz.toDouble())
    }

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

    // ---- Morse decoder (TinyCW), polled while its tab is shown
    private val _cw = MutableStateFlow(CwState())
    val cw: StateFlow<CwState> = _cw.asStateFlow()
    private var cwPoller: Job? = null

    fun watchCw(open: Boolean) {
        cwPoller?.cancel()
        if (!open) return
        cwPoller = viewModelScope.launch {
            while (isActive) {
                if (Engine.ready.value) _cw.value = Engine.cw()
                delay(if (_cw.value.enabled) 400 else 1500)
            }
        }
    }

    fun setCwEnabled(on: Boolean) = viewModelScope.launch {
        if (Engine.ready.value) _cw.value = Engine.cwSet("enabled", if (on) 1.0 else 0.0)
    }

    /** Listen to a decoded signal: CW mode on its frequency. */
    fun tuneCw(freqHz: Double) {
        if (state.value.mode != Mode.CW) setMode(Mode.CW)
        tune(freqHz)
    }

    // ---- Recorder (audio / baseband WAV into the app's storage), polled while its tab is shown
    private val _rec = MutableStateFlow(RecorderState())
    val rec: StateFlow<RecorderState> = _rec.asStateFlow()
    private val _recordings = MutableStateFlow<List<File>>(emptyList())
    val recordings: StateFlow<List<File>> = _recordings.asStateFlow()
    private var recPoller: Job? = null
    private var recPath = ""

    // Documents/LittleSDR/Audio or .../Baseband, following the recording mode
    private fun recDir(audio: Boolean) = Storage.dir(getApplication(), if (audio) Storage.AUDIO else Storage.BASEBAND)

    private suspend fun ensureRecPath(audio: Boolean = _rec.value.audio) {
        val p = recDir(audio).path
        if (p != recPath) {
            Engine.recorderPath(p)
            recPath = p
        }
    }

    fun watchRecorder(open: Boolean) {
        recPoller?.cancel()
        if (!open) return
        refreshRecordings()
        recPoller = viewModelScope.launch {
            while (isActive) {
                if (Engine.ready.value) {
                    ensureRecPath()
                    _rec.value = Engine.recorder()
                }
                delay(if (_rec.value.recording || _rec.value.audio) 250 else 1000)
            }
        }
    }

    fun setRecorder(key: String, value: Double) = viewModelScope.launch {
        if (!Engine.ready.value) return@launch
        val file = _rec.value.file
        if (key == "start") ensureRecPath(Engine.recorder().audio)
        _rec.value = Engine.recorderSet(key, value)
        if (key == "stop" && file.isNotEmpty()) Storage.scan(getApplication(), File(file))
        if (key == "stop" || key == "start") refreshRecordings()
    }

    /** Where the files go, as the user sees it in a file manager ("Documents/LittleSDR"). */
    fun storageLabel(): String = Storage.root(getApplication()).path
        .removePrefix(android.os.Environment.getExternalStorageDirectory().path).trimStart('/')

    fun refreshRecordings() = viewModelScope.launch {
        _recordings.value = withContext(Dispatchers.IO) {
            listOf(recDir(true), recDir(false))
                .flatMap { d -> d.listFiles { f -> f.extension == "wav" }.orEmpty().toList() }
                .sortedByDescending { it.name }
        }
    }

    // ---- SSTV: polled while the decoder is on (tab shown or not), finished images saved as PNG
    private val _sstv = MutableStateFlow(SstvState())
    val sstv: StateFlow<SstvState> = _sstv.asStateFlow()
    private val _sstvImage = MutableStateFlow<Bitmap?>(null)
    val sstvImage: StateFlow<Bitmap?> = _sstvImage.asStateFlow()
    private val _sstvGallery = MutableStateFlow<List<File>>(emptyList())
    val sstvGallery: StateFlow<List<File>> = _sstvGallery.asStateFlow()
    private var sstvPoller: Job? = null
    private val sstvDir: File get() = Storage.dir(getApplication(), Storage.SSTV) // Documents/LittleSDR/SSTV

    fun setSstvEnabled(on: Boolean) = viewModelScope.launch {
        if (!Engine.ready.value) return@launch
        _sstv.value = Engine.sstvSet("enabled", if (on) 1.0 else 0.0)
        sstvPoller?.cancel()
        sstvPoller = null
        if (on) sstvPoller = viewModelScope.launch { pollSstv() }
    }

    fun sstvStartManual(modeIndex: Int) = viewModelScope.launch { _sstv.value = Engine.sstvSet("start", modeIndex.toDouble()) }
    fun sstvFinish() = viewModelScope.launch { _sstv.value = Engine.sstvSet("stop", 0.0) }

    private suspend fun pollSstv() {
        var seenVersion = -1L
        var seenFinished = Engine.sstv().finished // images finished before this session are not saved again
        while (true) {
            val s = Engine.sstv()
            if (s.finished != seenFinished) {
                seenFinished = s.finished
                saveSstv(s)
            }
            if (s.version != seenVersion && s.width > 0) {
                seenVersion = s.version
                _sstvImage.value = toBitmap(Engine.sstvImage(0))
            }
            _sstv.value = s
            delay(if (s.receiving) 400 else 1000)
        }
    }

    private fun toBitmap(a: IntArray): Bitmap? {
        if (a.size < 2) return null
        val w = a[0]
        val h = a[1]
        if (w <= 0 || h <= 0 || a.size < 2 + w * h) return null
        return Bitmap.createBitmap(a, 2, w, w, h, Bitmap.Config.ARGB_8888)
    }

    private suspend fun saveSstv(s: SstvState) {
        val full = toBitmap(Engine.sstvImage(1)) ?: return
        val rows = s.finishedRows.coerceIn(1, full.height)
        val bmp = if (rows < full.height) Bitmap.createBitmap(full, 0, 0, full.width, rows) else full
        val stamp = SimpleDateFormat("yyyy-MM-dd_HH-mm-ss", Locale.ROOT).format(Date())
        val mhz = String.format(Locale.ROOT, "%.3fMHz", state.value.vfoFrequency / 1e6)
        val saved = runCatching {
            val file = File(sstvDir, "${stamp}_${s.finishedMode.replace(' ', '-')}_$mhz.png")
            withContext(Dispatchers.IO) { file.outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) } }
            file
        }
        saved.onSuccess { Storage.scan(getApplication(), it) }
            .onFailure { _error.value = "Image SSTV non enregistrée : ${it.message}" }
        refreshSstvGallery()
    }

    fun refreshSstvGallery() = viewModelScope.launch {
        _sstvGallery.value = withContext(Dispatchers.IO) {
            sstvDir.listFiles { f -> f.extension == "png" }?.sortedByDescending { it.name }.orEmpty()
        }
    }

    fun deleteFile(f: File, refresh: () -> Unit) = viewModelScope.launch {
        withContext(Dispatchers.IO) { f.delete() }
        refresh()
    }

    // ---- KiwiSDR server picker
    private val kiwiDirectory = KiwiDirectory(app)
    private val _kiwiServers = MutableStateFlow<List<KiwiServer>>(emptyList())
    val kiwiServers: StateFlow<List<KiwiServer>> = _kiwiServers.asStateFlow()
    private val _kiwiLoading = MutableStateFlow(false)
    val kiwiLoading: StateFlow<Boolean> = _kiwiLoading.asStateFlow()

    /** Operator position (locator from the settings), to sort receivers by distance. */
    fun operatorPosition(): GeoPoint? = Maidenhead.toLatLon(_advanced.value.operator.location)

    fun loadKiwiServers(force: Boolean = false) = viewModelScope.launch {
        _kiwiLoading.value = true
        _kiwiServers.value = kiwiDirectory.load(force)
        _kiwiLoading.value = false
        if (_kiwiServers.value.isEmpty()) _error.value = "Liste des KiwiSDR indisponible (connexion internet ?)"
    }

    private var kiwiJob: Job? = null

    /** KiwiSDR servers that turned the app away (their application slots taken): marked in the picker */
    private val kiwiPrefs = app.getSharedPreferences("kiwi", android.content.Context.MODE_PRIVATE)
    private val _kiwiRefused = MutableStateFlow(kiwiPrefs.getStringSet("refused", emptySet())!!.toSet())
    val kiwiRefused: StateFlow<Set<String>> = _kiwiRefused.asStateFlow()

    private fun currentKiwiSite(): String? =
        _sourceMenu.value.filterIsInstance<SmWidget.InputText>().firstOrNull { it.id == "kiwi_site" }?.value?.takeIf { it.isNotBlank() }

    private fun markKiwi(site: String, refused: Boolean) {
        val now = if (refused) _kiwiRefused.value + site else _kiwiRefused.value - site
        if (now == _kiwiRefused.value) return
        _kiwiRefused.value = now
        kiwiPrefs.edit().putStringSet("refused", now).apply()
    }

    init {
        // Reception that ends without the user stopping it: say why (server refusal, unplugged),
        // and remember KiwiSDR servers that turn applications away. A server that then plays
        // for 30 s is cleared again.
        viewModelScope.launch {
            var was = false
            var cleared: Job? = null
            state.map { it.playing }.distinctUntilChanged().collect { playing ->
                cleared?.cancel()
                if (playing) {
                    if (selectedSource.value == "KiwiSDR") cleared = launch {
                        delay(30_000)
                        if (state.value.playing) currentKiwiSite()?.let { markKiwi(it, false) }
                    }
                } else if (was && !userStopped) {
                    val why = Engine.sourceError()
                    _error.value = if (why.isBlank()) "Réception interrompue" else why
                    if (selectedSource.value == "KiwiSDR" && why.startsWith("Pas de place pour les applications")) {
                        currentKiwiSite()?.let { markKiwi(it, true) }
                    }
                }
                userStopped = false
                was = playing
            }
        }
    }

    /** Only the latest pick counts: an earlier, slower one is cancelled before it reaches the engine. */
    fun setKiwiSite(hostPort: String, name: String) {
        kiwiJob?.cancel()
        kiwiJob = viewModelScope.launch {
            val resolved = KiwiDirectory.resolve(hostPort.trim()) // cancelled here if a newer pick came
            if (resolved == null) {
                _error.value = "« $name » ne répond pas (récepteur éteint ou proxy kiwisdr.com indisponible)"
                return@launch
            }
            Engine.kiwiSetSite(resolved, name)
            _sourceMenu.value = Engine.sourceMenu()
        }
    }

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

    /** Band preset: mode, then bandwidth and step for that mode, then the frequency. */
    fun applyPreset(e: PresetEntry) {
        Engine.setMode(e.mode)
        Engine.setBandwidth(e.bandwidth)
        setStep(e.mode, e.step)
        tune(e.frequency)
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
