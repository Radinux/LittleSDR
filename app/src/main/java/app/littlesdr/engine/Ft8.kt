package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Mirror of engine::ft8Json() (cpp/modules/ft8_ext.cpp). */
@Serializable
data class Ft8State(
    val enabled: Boolean = false,
    val ft8: Ft8Decoder = Ft8Decoder(),
    val ft4: Ft8Decoder = Ft8Decoder(),
    val results: List<Ft8Result> = emptyList(),
) {
    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): Ft8State = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(Ft8State())
    }
}

@Serializable
data class Ft8Decoder(
    val processing: Boolean = true,
    val onFrequency: Boolean = false,
    /** Dial frequency (Hz) of the FT8/FT4 channel inside the displayed span, 0 if none */
    val dial: Long = 0,
    val lastDecodeCount: Int = 0,
    val decoding: Boolean = false,
    val error: String = "",
)

@Serializable
data class Ft8Result(
    val mode: String = "FT8",
    /** End of the decoded slot, ms since epoch (UTC) */
    val time: Long = 0,
    /** Audio offset inside the 3 kHz passband (Hz) */
    val audioHz: Long = 0,
    val band: String = "",
    val call: String = "",
    val message: String = "",
    val snr: Double = 0.0,
    val distanceKm: Double = 0.0,
    val country: String = "",
) {
    val isCq: Boolean get() = message.startsWith("CQ ")
    /** Start of the slot (FT8 15 s, FT4 7.5 s) */
    val slotStart: Long get() = time - if (mode == "FT4") 7_500 else 15_000
}

/** Standard dial frequencies (Hz), same lists as the decoder module. */
object Ft8Bands {
    val FT8 = listOf(
        "160 m" to 1_840_000L, "80 m" to 3_573_000L, "60 m" to 5_357_000L, "40 m" to 7_074_000L,
        "30 m" to 10_136_000L, "20 m" to 14_074_000L, "17 m" to 18_100_000L, "15 m" to 21_074_000L,
        "12 m" to 24_915_000L, "10 m" to 28_074_000L, "6 m" to 50_323_000L, "2 m" to 144_174_000L,
    )
    val FT4 = listOf(
        "40 m" to 7_047_500L, "30 m" to 10_140_000L, "20 m" to 14_080_000L,
        "17 m" to 18_104_000L, "15 m" to 21_140_000L, "10 m" to 28_180_000L,
    )
}
