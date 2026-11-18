package app.littlesdr.engine

/** What a frequency range is allocated to, drawn as a colored strip under the waterfall. */
enum class BandKind(val label: String) {
    HAM("Radioamateur"),
    BROADCAST("Radiodiffusion"),
    CB("CB / usage libre"),
    AIR("Aviation"),
    MARINE("Marine"),
}

data class BandSegment(val startHz: Double, val endHz: Double, val kind: BandKind, val label: String)

/**
 * Main allocations per ITU region (1 = Europe, Africa, Middle East; 2 = Americas;
 * 3 = Asia, Oceania). Simplified from the ITU Radio Regulations article 5: national band
 * plans differ in the details.
 */
object BandPlan {
    val REGIONS = listOf(
        1 to "Région 1 · Europe, Afrique, Moyen-Orient",
        2 to "Région 2 · Amériques",
        3 to "Région 3 · Asie, Océanie",
    )

    private fun k(v: Double) = v * 1e3
    private fun m(v: Double) = v * 1e6

    /** [regions]: the ITU regions where the segment applies. */
    private class Entry(val start: Double, val end: Double, val kind: BandKind, val label: String, val regions: Set<Int> = ALL)

    private val ALL = setOf(1, 2, 3)
    private val R1 = setOf(1)
    private val R2 = setOf(2)
    private val R3 = setOf(3)
    private val R13 = setOf(1, 3)
    private val R23 = setOf(2, 3)

    private val ENTRIES = listOf(
        // ---- Amateur radio
        Entry(k(135.7), k(137.8), BandKind.HAM, "2200 m"),
        Entry(k(472.0), k(479.0), BandKind.HAM, "630 m"),
        Entry(k(1810.0), k(2000.0), BandKind.HAM, "160 m", R1),
        Entry(k(1800.0), k(2000.0), BandKind.HAM, "160 m", R23),
        Entry(k(3500.0), k(3800.0), BandKind.HAM, "80 m", R1),
        Entry(k(3500.0), k(4000.0), BandKind.HAM, "80 m", R2),
        Entry(k(3500.0), k(3900.0), BandKind.HAM, "80 m", R3),
        Entry(k(5351.5), k(5366.5), BandKind.HAM, "60 m"),
        Entry(k(7000.0), k(7200.0), BandKind.HAM, "40 m", R13),
        Entry(k(7000.0), k(7300.0), BandKind.HAM, "40 m", R2),
        Entry(k(10100.0), k(10150.0), BandKind.HAM, "30 m"),
        Entry(k(14000.0), k(14350.0), BandKind.HAM, "20 m"),
        Entry(k(18068.0), k(18168.0), BandKind.HAM, "17 m"),
        Entry(k(21000.0), k(21450.0), BandKind.HAM, "15 m"),
        Entry(k(24890.0), k(24990.0), BandKind.HAM, "12 m"),
        Entry(k(28000.0), k(29700.0), BandKind.HAM, "10 m"),
        Entry(m(50.0), m(52.0), BandKind.HAM, "6 m", R1),
        Entry(m(50.0), m(54.0), BandKind.HAM, "6 m", R23),
        Entry(m(70.0), m(70.5), BandKind.HAM, "4 m", R1),
        Entry(m(144.0), m(146.0), BandKind.HAM, "2 m", R1),
        Entry(m(144.0), m(148.0), BandKind.HAM, "2 m", R23),
        Entry(m(222.0), m(225.0), BandKind.HAM, "1,25 m", R2),
        Entry(m(430.0), m(440.0), BandKind.HAM, "70 cm", R13),
        Entry(m(420.0), m(450.0), BandKind.HAM, "70 cm", R2),
        Entry(m(1240.0), m(1300.0), BandKind.HAM, "23 cm"),

        // ---- Broadcasting
        Entry(k(148.5), k(283.5), BandKind.BROADCAST, "GO", R1),
        Entry(k(526.5), k(1606.5), BandKind.BROADCAST, "PO", R13),
        Entry(k(525.0), k(1705.0), BandKind.BROADCAST, "PO", R2),
        Entry(k(2300.0), k(2495.0), BandKind.BROADCAST, "120 m"),
        Entry(k(3200.0), k(3400.0), BandKind.BROADCAST, "90 m"),
        Entry(k(3950.0), k(4000.0), BandKind.BROADCAST, "75 m", R1),
        Entry(k(3900.0), k(4000.0), BandKind.BROADCAST, "75 m", R3),
        Entry(k(4750.0), k(5060.0), BandKind.BROADCAST, "60 m"),
        Entry(k(5900.0), k(6200.0), BandKind.BROADCAST, "49 m"),
        Entry(k(7200.0), k(7450.0), BandKind.BROADCAST, "41 m", R13),
        Entry(k(7300.0), k(7450.0), BandKind.BROADCAST, "41 m", R2),
        Entry(k(9400.0), k(9900.0), BandKind.BROADCAST, "31 m"),
        Entry(k(11600.0), k(12100.0), BandKind.BROADCAST, "25 m"),
        Entry(k(13570.0), k(13870.0), BandKind.BROADCAST, "22 m"),
        Entry(k(15100.0), k(15800.0), BandKind.BROADCAST, "19 m"),
        Entry(k(17480.0), k(17900.0), BandKind.BROADCAST, "16 m"),
        Entry(k(18900.0), k(19020.0), BandKind.BROADCAST, "15 m"),
        Entry(k(21450.0), k(21850.0), BandKind.BROADCAST, "13 m"),
        Entry(k(25670.0), k(26100.0), BandKind.BROADCAST, "11 m"),
        Entry(m(87.5), m(108.0), BandKind.BROADCAST, "FM", R13),
        Entry(m(88.0), m(108.0), BandKind.BROADCAST, "FM", R2),
        Entry(m(174.0), m(240.0), BandKind.BROADCAST, "DAB+ / TV", R1),
        Entry(m(174.0), m(216.0), BandKind.BROADCAST, "TV VHF", R2),
        Entry(m(470.0), m(694.0), BandKind.BROADCAST, "TV UHF", R1),
        Entry(m(470.0), m(608.0), BandKind.BROADCAST, "TV UHF", R2),
        Entry(m(470.0), m(698.0), BandKind.BROADCAST, "TV UHF", R3),

        // ---- Licence-free radio
        Entry(k(26965.0), k(27405.0), BandKind.CB, "CB"),
        Entry(m(446.0), m(446.2), BandKind.CB, "PMR446", R1),
        Entry(m(433.05), m(434.79), BandKind.CB, "LPD433", R1),
        Entry(m(462.55), m(467.725), BandKind.CB, "FRS / GMRS", R2),
        Entry(m(476.425), m(477.4125), BandKind.CB, "CB UHF", R3),

        // ---- Aeronautical and maritime
        Entry(m(108.0), m(137.0), BandKind.AIR, "Aviation"),
        Entry(m(156.0), m(162.025), BandKind.MARINE, "VHF marine"),
    )

    private val byRegion = (1..3).associateWith { r ->
        ENTRIES.filter { r in it.regions }.map { BandSegment(it.start, it.end, it.kind, it.label) }.sortedBy { it.startHz }
    }

    fun segments(region: Int): List<BandSegment> = byRegion[region] ?: byRegion.getValue(1)

    /** Segments overlapping [lo, hi] Hz. */
    fun visible(region: Int, lo: Double, hi: Double): List<BandSegment> =
        segments(region).filter { it.endHz > lo && it.startHz < hi }
}
