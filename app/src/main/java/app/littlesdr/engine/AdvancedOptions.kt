package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Mirror of engine::optionsJson() (cpp/engine.cpp). Keys for setOption: "<section>.<field>". */
@Serializable
data class AdvancedOptions(
    val input: InputOptions = InputOptions(),
    val fft: FftOptions = FftOptions(),
    val audio: AudioOptions = AudioOptions(),
    val demod: DemodOptions = DemodOptions(),
    val operator: OperatorOptions = OperatorOptions(),
) {
    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): AdvancedOptions = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(AdvancedOptions())
    }
}

@Serializable
data class OperatorOptions(
    /** Maidenhead locator (e.g. JN18), used for distances to FT8 stations */
    val location: String = "",
    val callsign: String = "",
)

@Serializable
data class InputOptions(
    val decimation: Int = 1,
    val dcBlocking: Boolean = false,
    val invertIQ: Boolean = false,
    /** Frequency converter offset (Hz), added to the hardware frequency */
    val offset: Double = 0.0,
)

@Serializable
data class FftOptions(
    val size: Int = 8192,
    val rate: Float = 25f,
    /** 0 rectangular, 1 Blackman, 2 Nuttall */
    val window: Int = 2,
    val smoothing: Float = 0.4f,
    val peakHold: Boolean = false,
    /** dB per second */
    val peakDecay: Float = 10f,
)

@Serializable
data class AudioOptions(
    /** AudioDeviceInfo id, 0 = system default route */
    val device: Int = 0,
    /** Buffer size in bursts */
    val latency: Int = 4,
)

/** Options of the current demodulator (radio module, via cpp/modules/radio_ext.cpp). */
@Serializable
data class DemodOptions(
    val demod: String = "",
    val deempAllowed: Boolean = false,
    val deemp: Int = 0,
    val deempModes: List<String> = emptyList(),
    val ifnrAllowed: Boolean = false,
    val ifnr: Boolean = false,
    val ifnrPreset: Int = 0,
    val ifnrPresets: List<String> = emptyList(),
    val nbAllowed: Boolean = false,
    val nb: Boolean = false,
    val nbLevel: Float = 10f,
    val agc: Boolean = false,
    val agcAttack: Float = 50f,
    val agcDecay: Float = 5f,
    /** Broadcast FM stereo: only for WFM */
    val stereoAllowed: Boolean = false,
    val stereo: Boolean = false,
)

/** Frequency converters, same values as SDR++'s named offsets. */
enum class Converter(val label: String, val offsetHz: Double) {
    None("Aucun", 0.0),
    HamItUp("Ham-It-Up (+125 MHz)", 125e6),
    SpyVerter("SpyVerter (+120 MHz)", 120e6),
    LnbKu9750("LNB Ku 9 750 MHz", -9750e6),
    LnbKu10700("LNB Ku 10 700 MHz", -10700e6),
    Manual("Manuel", Double.NaN);

    companion object {
        fun of(offset: Double): Converter =
            entries.firstOrNull { !it.offsetHz.isNaN() && it.offsetHz == offset } ?: Manual
    }
}
