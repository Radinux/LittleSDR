package app.littlesdr.engine

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.Executors

/** Demodulator modes, same order as SDR++ RADIO_IFACE_MODE_*. */
enum class Mode(val label: String) {
    NFM("NFM"), WFM("WFM"), AM("AM"), DSB("DSB"), USB("USB"), CW("CW"), LSB("LSB"), RAW("RAW");

    /** Sensible bandwidth slider range (Hz) per mode. */
    val bandwidthRange: ClosedFloatingPointRange<Float>
        get() = when (this) {
            NFM -> 1_000f..50_000f
            WFM -> 50_000f..250_000f
            AM -> 1_000f..30_000f
            DSB -> 1_000f..20_000f
            USB, LSB -> 500f..6_000f
            CW -> 50f..1_000f
            RAW -> 1_000f..250_000f
        }

    val isFm: Boolean get() = this == NFM || this == WFM

    /** Bandwidth slider increment (Hz): 1 kHz, finer where the range is narrow. */
    val bandwidthStep: Float
        get() = when (this) {
            USB, LSB -> 100f
            CW -> 10f
            else -> 1_000f
        }

    /** Default tuning step (Hz). */
    val defaultStep: Double
        get() = when (this) {
            WFM -> 100_000.0
            NFM -> 12_500.0
            AM -> 5_000.0
            DSB, USB, LSB -> 100.0
            CW -> 10.0
            RAW -> 1_000.0
        }

    companion object {
        fun of(id: Int) = entries.getOrNull(id)
    }
}

data class RadioState(
    val vfoFrequency: Double = 100_000_000.0,
    val centerFrequency: Double = 100_000_000.0,
    val sampleRate: Double = 0.0,
    val vfoOffset: Double = 0.0,
    val vfoBandwidth: Double = 0.0,
    val mode: Mode? = null,
    val playing: Boolean = false,
    val squelchEnabled: Boolean = false,
    val squelchLevel: Float = -100f,
    val volume: Float = 1f,
)

/**
 * Owner of the native engine. All native calls run on one dedicated thread, which plays
 * the role of SDR++'s UI thread (the engine is not designed for concurrent control calls).
 */
object Engine {
    private const val TAG = "LittleSDR"
    private const val FFT_SIZE = 8192

    private val thread = Executors.newSingleThreadExecutor { r -> Thread(r, "sdr-engine") }
    private val dispatcher = thread.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + dispatcher)

    /** Provided by the app so native code can query it (KiwiSDR/battery widgets). */
    var batteryLevel: (() -> Int)? = null

    private val _ready = MutableStateFlow(false)
    val ready: StateFlow<Boolean> = _ready.asStateFlow()

    private val _state = MutableStateFlow(RadioState())
    val state: StateFlow<RadioState> = _state.asStateFlow()

    /** Peak level in the VFO passband. Separate from [state] so only the meter recomposes. */
    private val _signalDb = MutableStateFlow(-150f)
    val signalDb: StateFlow<Float> = _signalDb.asStateFlow()

    private val _sources = MutableStateFlow<List<String>>(emptyList())
    val sources: StateFlow<List<String>> = _sources.asStateFlow()

    private val _selectedSource = MutableStateFlow("")
    val selectedSource: StateFlow<String> = _selectedSource.asStateFlow()

    // Latest requested frequency; intermediate values are dropped while the engine is busy
    private val tuneTarget = MutableStateFlow<Double?>(null)
    @Volatile private var tuneRequestedAt = 0L
    // While a slider is moving, the UI value wins over (possibly stale) polled values
    @Volatile private var bandwidthHoldUntil = 0L
    @Volatile private var squelchHoldUntil = 0L

    fun init(context: Context) {
        if (_ready.value) return
        val root = File(context.filesDir, "sdr").absolutePath
        val cache = context.cacheDir.absolutePath
        scope.launch {
            installResources(context, File(root, "res"))
            val ok = NativeBridge.nativeInit(root, cache, FFT_SIZE)
            if (!ok) {
                Log.e(TAG, "native engine init failed")
                return@launch
            }
            refreshSources()
            _ready.value = true
            launch { tuneTarget.filterNotNull().collect { NativeBridge.tune(it) } }
            while (isActive) {
                pollState()
                delay(100)
            }
        }
    }

    /** Copies bundled engine resources (assets/res/...) once per app version. */
    private fun installResources(context: Context, dest: File) {
        val version = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).longVersionCode }.getOrDefault(0L)
        val marker = File(dest, ".installed")
        if (marker.exists() && marker.readText() == version.toString()) return
        fun copy(assetPath: String, target: File) {
            val children = context.assets.list(assetPath).orEmpty()
            if (children.isEmpty()) {
                target.parentFile?.mkdirs()
                context.assets.open(assetPath).use { input -> target.outputStream().use { input.copyTo(it) } }
            } else {
                children.forEach { copy("$assetPath/$it", File(target, it)) }
            }
        }
        runCatching {
            copy("res", dest)
            marker.writeText(version.toString())
        }.onFailure { Log.e(TAG, "resource install failed", it) }
    }

    private fun pollState() {
        val s = NativeBridge.state()
        val now = System.currentTimeMillis()
        val cur = _state.value
        val holdBw = now < bandwidthHoldUntil
        val holdSq = now < squelchHoldUntil
        _state.value = RadioState(
            vfoFrequency = tuneTarget.value ?: s[0],
            centerFrequency = s[1],
            sampleRate = s[2],
            vfoOffset = s[3],
            vfoBandwidth = if (holdBw) cur.vfoBandwidth else s[4],
            mode = Mode.of(s[6].toInt()),
            playing = s[7] != 0.0,
            squelchEnabled = if (holdSq) cur.squelchEnabled else s[8] != 0.0,
            squelchLevel = if (holdSq) cur.squelchLevel else s[9].toFloat(),
            volume = s[10].toFloat(),
        )
        // Rounded to 1 dB: avoids recomposing the meter for invisible changes
        _signalDb.value = kotlin.math.round(s[5].toFloat())
        // Once the engine caught up with the last tune request (or gave up), stop overriding its value
        val target = tuneTarget.value
        if (target != null &&
            (kotlin.math.abs(s[0] - target) < 0.5 || System.currentTimeMillis() - tuneRequestedAt > 400)
        ) {
            tuneTarget.compareAndSet(target, null)
        }
    }

    private fun refreshSources() {
        _sources.value = NativeBridge.sourceNames().toList()
        _selectedSource.value = NativeBridge.selectedSource()
    }

    suspend fun <T> call(block: () -> T): T = withContext(dispatcher) { block() }
    fun post(block: () -> Unit) {
        scope.launch { if (_ready.value) block() }
    }

    // ---------------------------------------------------------------- controls
    fun tune(hz: Double) {
        if (hz <= 0) return
        tuneRequestedAt = System.currentTimeMillis()
        tuneTarget.value = hz
        _state.value = _state.value.copy(vfoFrequency = hz)
    }

    fun tuneCenter(hz: Double) = post { NativeBridge.tuneCenter(hz) }

    suspend fun start(): Boolean = call {
        NativeBridge.start().also { ok -> if (ok) _state.value = _state.value.copy(playing = true) }
    }

    /**
     * Stops the radio and ends the process (app swiped away / closed while idle): frees the
     * native engine, the USB device and the audio stream instead of lingering in RAM.
     * The short delay lets the engine's config autosave threads flush to disk.
     */
    fun shutdownAndExit() {
        scope.launch {
            if (_ready.value) NativeBridge.stop()
            delay(1500)
            android.os.Process.killProcess(android.os.Process.myPid())
        }
    }
    suspend fun stop() = call { NativeBridge.stop() }

    fun setMode(mode: Mode) = post { NativeBridge.setMode(mode.ordinal) }
    // Optimistic state updates: the overlay (VFO passband, squelch line) follows the slider
    // immediately instead of waiting for the next poll; the poll then confirms the engine's value.
    fun setBandwidth(hz: Float) {
        bandwidthHoldUntil = System.currentTimeMillis() + 300
        _state.value = _state.value.copy(vfoBandwidth = hz.toDouble())
        post { NativeBridge.setBandwidth(hz) }
    }

    fun setSquelch(enabled: Boolean, levelDb: Float) {
        squelchHoldUntil = System.currentTimeMillis() + 300
        _state.value = _state.value.copy(squelchEnabled = enabled, squelchLevel = levelDb)
        post { NativeBridge.setSquelch(enabled, levelDb) }
    }
    fun setVolume(v: Float) = post { NativeBridge.setVolume(v) }
    fun setView(viewBandwidth: Double, viewOffset: Double) = post { NativeBridge.setView(viewBandwidth, viewOffset) }

    suspend fun options(): AdvancedOptions = call { AdvancedOptions.parse(NativeBridge.options()) }

    /** Applies one advanced option and returns the refreshed options. */
    suspend fun setOption(key: String, value: Double): AdvancedOptions = call {
        NativeBridge.setOption(key, value)
        AdvancedOptions.parse(NativeBridge.options())
    }

    suspend fun setOptionString(key: String, value: String): AdvancedOptions = call {
        NativeBridge.setOptionString(key, value)
        AdvancedOptions.parse(NativeBridge.options())
    }

    suspend fun ft8(): Ft8State = call { Ft8State.parse(NativeBridge.ft8Json()) }
    suspend fun ft8Set(key: String, value: Double): Ft8State = call {
        NativeBridge.ft8Set(key, value)
        Ft8State.parse(NativeBridge.ft8Json())
    }

    suspend fun selectSource(name: String) = call {
        NativeBridge.selectSource(name)
        refreshSources()
    }

    /** Reads the selected source's settings, optionally applying one user action first. */
    suspend fun sourceMenu(action: SmAction? = null): List<SmWidget> = call {
        val json = if (action == null) {
            NativeBridge.sourceMenu("", 0, false, 0, 0f, "")
        } else {
            NativeBridge.sourceMenu(action.id, action.type, action.b, action.i, action.f, action.s)
        }
        SmGui.parse(json)
    }

    /** Plugged USB receiver: hand its fd to the drivers, select its source and rescan devices. */
    suspend fun attachUsb(fd: Int, vid: Int, pid: Int, sourceName: String?) = call {
        NativeBridge.setUsbDevice(fd, vid, pid)
        if (sourceName != null && !_state.value.playing && sourceName in NativeBridge.sourceNames()) {
            NativeBridge.selectSource(sourceName)
            refreshSources()
            // Every USB source module exposes a "Refresh" button that rescans devices
            val refresh = SmGui.parse(NativeBridge.sourceMenu("", 0, false, 0, 0f, ""))
                .filterIsInstance<SmWidget.Button>()
                .firstOrNull { it.label.equals("Refresh", ignoreCase = true) }
            if (refresh != null) NativeBridge.sourceMenu(refresh.id, SmGui.TYPE_BOOL, true, 0, 0f, "")
        }
    }

    suspend fun detachUsb() = call {
        if (_state.value.playing) NativeBridge.stop()
        NativeBridge.clearUsbDevice()
    }
}
