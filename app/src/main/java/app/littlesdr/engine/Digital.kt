package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

private val json = Json { ignoreUnknownKeys = true }

/** One service of a DAB ensemble */
@Serializable
data class DabService(val sid: String = "", val label: String = "", val dabPlus: Boolean = true)

/** DAB+ or DRM reception (engine digitalJson) */
@Serializable
data class DigitalState(
    /** "dab", "drm" or "none" */
    val kind: String = "none",
    val synced: Boolean = false,
    val snr: Float = 0f,
    /** DRM: reception quality 0..1 (share of recent frames decoded) */
    val quality: Float = 0f,
    val ensemble: String = "",
    val services: List<DabService> = emptyList(),
    val playing: String? = null,
    /** DAB: bit rate of the service being played, kbit/s (0: unknown) */
    val bitrate: Int = 0,
    /** DAB: SId (hex) of the service played or asked for */
    val sid: String? = null,
    /** DAB: other frequencies (Hz) carrying that service, as the multiplex announces them */
    val af: List<Long> = emptyList(),
    /** DAB: FM frequencies (Hz) of the same programme */
    val fm: List<Long> = emptyList(),
    /** DAB programme text (DLS), or DRM text message */
    val text: String = "",
    /** Slideshow picture counter (0: none yet) */
    val slide: Long = 0,
    /** Name of the slideshow picture as the station sent it ("logo.png", "cover.jpg"...) */
    val slideName: String = "",
    /** DRM station label, and label + audio coding */
    val station: String = "",
    val info: String = "",
    val status: String = "",
) {
    companion object {
        fun parse(s: String): DigitalState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(DigitalState())
    }
}

/** Meteor-M LRPT reception (engine lrptJson) */
@Serializable
data class LrptState(
    val active: Boolean = false,
    val locked: Boolean = false,
    val carrier: Double = 0.0,
    val eye: Float = 0f,
    val oqpsk: Boolean = true,
    val framesOk: Long = 0,
    val framesBad: Long = 0,
    val packets: Long = 0,
    val channels: List<Int> = emptyList(),
    val lines: Int = 0,
) {
    companion object {
        fun parse(s: String): LrptState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(LrptState())
    }
}

/** A DAB channel of band III */
data class DabChannel(val name: String, val hz: Double)

/** Band III, 5A to 13F (the N channels are not used in Europe) */
val DAB_CHANNELS: List<DabChannel> = listOf(
    "5A" to 174_928, "5B" to 176_640, "5C" to 178_352, "5D" to 180_064,
    "6A" to 181_936, "6B" to 183_648, "6C" to 185_360, "6D" to 187_072,
    "7A" to 188_928, "7B" to 190_640, "7C" to 192_352, "7D" to 194_064,
    "8A" to 195_936, "8B" to 197_648, "8C" to 199_360, "8D" to 201_072,
    "9A" to 202_928, "9B" to 204_640, "9C" to 206_352, "9D" to 208_064,
    "10A" to 209_936, "10B" to 211_648, "10C" to 213_360, "10D" to 215_072,
    "11A" to 216_928, "11B" to 218_640, "11C" to 220_352, "11D" to 222_064,
    "12A" to 223_936, "12B" to 225_648, "12C" to 227_360, "12D" to 229_072,
    "13A" to 230_784, "13B" to 232_496, "13C" to 234_208, "13D" to 235_776, "13E" to 237_488, "13F" to 239_200,
).map { (n, khz) -> DabChannel(n, khz * 1_000.0) }

fun dabChannelAt(hz: Double): DabChannel? = DAB_CHANNELS.firstOrNull { kotlin.math.abs(it.hz - hz) < 1_000 }

/** A shortwave DRM frequency to try */
data class DrmPreset(val name: String, val hz: Double)

/** A DRM station received here: its name, frequency, and when (ms) it was last heard */
@Serializable
data class DrmHeard(val name: String, val hz: Double, val last: Long)

/** CNR-1 (China National Radio) broadcasts DRM most of the UTC day on these */
val DRM_PRESETS: List<DrmPreset> = listOf(6_030, 9_655, 13_790, 15_290, 15_760, 17_700, 21_590)
    .map { DrmPreset("CNR-1", it * 1_000.0) }

/** An AM broadcast band where DRM can be heard (ITU allocations, kHz edges) */
data class HfBand(val name: String, val lo: Double, val hi: Double) {
    operator fun contains(hz: Double) = hz in lo..hi
    val centre: Double get() = Math.round((lo + hi) / 2 / 5_000) * 5_000.0
}

/** Long and medium wave, then the shortwave broadcast bands, low to high */
val DRM_BANDS: List<HfBand> = listOf(
    "GO" to (148.5 to 283.5), "PO" to (526.5 to 1_606.5),
    "120 m" to (2_300.0 to 2_495.0), "90 m" to (3_200.0 to 3_400.0), "75 m" to (3_900.0 to 4_000.0),
    "60 m" to (4_750.0 to 5_060.0), "49 m" to (5_900.0 to 6_200.0), "41 m" to (7_200.0 to 7_450.0),
    "31 m" to (9_400.0 to 9_900.0), "25 m" to (11_600.0 to 12_100.0), "22 m" to (13_570.0 to 13_870.0),
    "19 m" to (15_100.0 to 15_800.0), "16 m" to (17_480.0 to 17_900.0), "15 m" to (18_900.0 to 19_020.0),
    "13 m" to (21_450.0 to 21_850.0), "11 m" to (25_670.0 to 26_100.0),
).map { (n, r) -> HfBand(n, r.first * 1_000.0, r.second * 1_000.0) }

fun hfBandAt(hz: Double): HfBand? = DRM_BANDS.firstOrNull { hz in it }

/** A Meteor-M LRPT downlink */
data class LrptSatellite(val name: String, val hz: Double)

/** Meteor-M N2-3 / N2-4 send LRPT (72 ksym/s OQPSK) on 137.9 MHz, sometimes 137.1 MHz */
val LRPT_SATELLITES: List<LrptSatellite> = listOf(
    LrptSatellite("Meteor-M · 137,900 MHz", 137_900_000.0),
    LrptSatellite("Meteor-M · 137,100 MHz", 137_100_000.0),
)
