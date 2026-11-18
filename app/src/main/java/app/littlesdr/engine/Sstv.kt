package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Mirror of engine::sstvJson() (cpp/engine.cpp, decoder in cpp/sstv/). */
@Serializable
data class SstvState(
    val enabled: Boolean = false,
    val receiving: Boolean = false,
    /** Mode of the current (or last) image, e.g. "PD 120" */
    val mode: String = "",
    /** Decoded image rows */
    val line: Int = 0,
    val width: Int = 0,
    val height: Int = 0,
    /** Changes whenever the current image does */
    val version: Long = 0,
    /** Number of finished images (complete or signal lost) */
    val finished: Long = 0,
    val finishedRows: Int = 0,
    val finishedMode: String = "",
    val lastComplete: Boolean = false,
    /** Smoothed audio frequency (Hz) */
    val freq: Double = 0.0,
    /** Clock deviation measured on the sync pulses */
    val slantPpm: Double = 0.0,
    /** Mode names, index = manual start argument */
    val modes: List<String> = emptyList(),
) {
    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): SstvState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(SstvState())
    }
}
