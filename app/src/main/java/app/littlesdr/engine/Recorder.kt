package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Mirror of the recorder state (cpp/modules/recorder_ext.cpp). */
@Serializable
data class RecorderState(
    val recording: Boolean = false,
    /** 0 = baseband (IQ, 500 kHz around the tuned frequency at most), 1 = audio */
    val mode: Int = 1,
    /** Audio files: stereo (2 channels) or mono */
    val stereo: Boolean = true,
    val file: String = "",
    val seconds: Double = 0.0,
    val bytes: Double = 0.0,
    val samplerate: Double = 0.0,
    /** Peak level of the audio since the last poll, dB (left, right) */
    val level: List<Float> = emptyList(),
    /** Baseband recording if started now: IQ rate and center frequency (Hz) */
    val bbRate: Double = 0.0,
    val bbCenter: Double = 0.0,
) {
    val audio: Boolean get() = mode == 1

    companion object {
        const val MODE_BASEBAND = 0
        const val MODE_AUDIO = 1
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): RecorderState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(RecorderState())
    }
}
