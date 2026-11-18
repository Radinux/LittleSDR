package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Mirror of engine::cwJson() (cpp/modules/tinycw_ext.cpp): TinyCW Morse decoder. */
@Serializable
data class CwState(
    /** Model loaded (weights are embedded, so normally always true) */
    val ready: Boolean = false,
    val enabled: Boolean = false,
    /** Amateur band whose CW segment is in view, e.g. "40m" */
    val band: String = "",
    /** A CW segment is in view: whole-band decoding only runs there */
    val inBand: Boolean = false,
    val lo: Double = 0.0,
    val hi: Double = 0.0,
    val channels: Int = 0,
    val busy: Double = 0.0,
    val focus: CwFocus = CwFocus(),
    val lines: List<CwLine> = emptyList(),
) {
    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): CwState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(CwState())
    }
}

/** Text of the tuned frequency (CW mode only). */
@Serializable
data class CwFocus(val active: Boolean = false, val text: String = "")

/** Latest text of one keyed signal inside the band segment. */
@Serializable
data class CwLine(val f: Double = 0.0, val text: String = "", val fresh: String = "")
