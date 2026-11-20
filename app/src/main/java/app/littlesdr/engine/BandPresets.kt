package app.littlesdr.engine

/** One place to tune to: frequency + demodulator settings. */
data class PresetEntry(
    val label: String,
    val frequency: Double,
    val mode: Mode,
    val bandwidth: Float,
    val step: Double,
)

/** A band preset: either one entry, or a list of sub-bands / channels to pick from. */
data class BandPreset(
    val name: String,
    val description: String,
    val entries: List<PresetEntry>,
    /** Entries are numbered channels (CB, PMR) rather than named sub-bands */
    val channels: Boolean = false,
    /** Entry tuned when the band itself is chosen */
    val defaultIndex: Int = 0,
    /** Frequencies (Hz) that belong to this band, to find it from wherever the VFO is */
    val range: ClosedFloatingPointRange<Double> = entries.minOf { it.frequency }..entries.maxOf { it.frequency },
)

object BandPresets {
    // CEPT CB channels 1..40 (note the historical 23/24/25 ordering)
    private val CB_MHZ = doubleArrayOf(
        26.965, 26.975, 26.985, 27.005, 27.015, 27.025, 27.035, 27.055, 27.065, 27.075,
        27.085, 27.105, 27.115, 27.125, 27.135, 27.155, 27.165, 27.175, 27.185, 27.205,
        27.215, 27.225, 27.255, 27.235, 27.245, 27.265, 27.275, 27.285, 27.295, 27.305,
        27.315, 27.325, 27.335, 27.345, 27.355, 27.365, 27.375, 27.385, 27.395, 27.405,
    )

    val ALL: List<BandPreset> = listOf(
        BandPreset(
            "FM", "Radiodiffusion FM · 87,5 – 108 MHz",
            listOf(PresetEntry("FM", 100.0e6, Mode.WFM, 150_000f, 100_000.0)),
            range = 87.5e6..108e6,
        ),
        BandPreset(
            "LW", "Grandes ondes · 153 – 279 kHz",
            listOf(PresetEntry("LW", 162e3, Mode.AM, 9_000f, 9_000.0)),
            range = 148.5e3..283.5e3,
        ),
        BandPreset(
            "MW", "Petites ondes · 531 – 1 602 kHz",
            listOf(PresetEntry("MW", 999e3, Mode.AM, 9_000f, 9_000.0)),
            range = 526.5e3..1_606.5e3,
        ),
        BandPreset(
            "HF", "Ondes courtes · bandes de radiodiffusion",
            listOf(
                "49 m" to 6.0e6, "41 m" to 7.3e6, "31 m" to 9.6e6, "25 m" to 11.8e6,
                "19 m" to 15.4e6, "16 m" to 17.7e6, "13 m" to 21.6e6,
            ).map { (n, f) -> PresetEntry(n, f, Mode.AM, 9_000f, 5_000.0) },
            range = 1.6e6..30e6, // all of shortwave (CB, narrower, wins inside it)
        ),
        BandPreset(
            "CB", "Citizen Band · 40 canaux · 26,965 – 27,405 MHz",
            CB_MHZ.mapIndexed { i, f -> PresetEntry("${i + 1}", f * 1e6, Mode.NFM, 10_000f, 10_000.0) },
            channels = true,
            defaultIndex = 18, // channel 19
            range = 26.96e6..27.41e6,
        ),
        BandPreset(
            "VHF", "Aviation, marine, radioamateur 2 m",
            listOf(
                PresetEntry("Aviation", 121.5e6, Mode.AM, 8_330f, 8_333.0),
                PresetEntry("Marine · canal 16", 156.8e6, Mode.NFM, 12_500f, 25_000.0),
                PresetEntry("Radioamateur 2 m", 145.5e6, Mode.NFM, 12_500f, 12_500.0),
            ),
            range = 108e6..174e6,
        ),
        BandPreset(
            "UHF", "Radioamateur 70 cm, LPD433",
            listOf(
                PresetEntry("Radioamateur 70 cm", 433.5e6, Mode.NFM, 12_500f, 12_500.0),
                PresetEntry("LPD433", 433.075e6, Mode.NFM, 12_500f, 25_000.0),
            ),
            range = 400e6..470e6, // PMR, narrower, wins inside it
        ),
        BandPreset(
            "PMR", "PMR446 · 16 canaux · 446,006 – 446,194 MHz",
            (0 until 16).map { i -> PresetEntry("${i + 1}", 446.00625e6 + i * 12_500.0, Mode.NFM, 12_500f, 12_500.0) },
            channels = true,
            range = 446.0e6..446.2e6,
        ),
    )

    /** The band the VFO is in: an exact preset first, else the narrowest band around it */
    fun at(hz: Double): BandPreset? =
        ALL.firstOrNull { b -> b.entries.any { kotlin.math.abs(it.frequency - hz) < 1.0 } }
            ?: ALL.filter { hz in it.range }.minByOrNull { it.range.endInclusive - it.range.start }
}
