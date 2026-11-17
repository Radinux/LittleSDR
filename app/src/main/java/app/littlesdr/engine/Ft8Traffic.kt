package app.littlesdr.engine

/** A point on the map. */
data class GeoPoint(val lat: Double, val lon: Double)

data class Ft8Station(
    val call: String,
    val pos: GeoPoint,
    /** Position from a Maidenhead locator (precise) rather than the country centroid */
    val precise: Boolean,
    val lastHeard: Long,
    val callingCq: Boolean,
    val snr: Double,
    /** Decoded on air (false = only seen as a recipient) */
    val heard: Boolean,
)

/** One transmission: [from] sends to [to] (CQ has no recipient). */
data class Ft8Link(val from: String, val to: String, val time: Long)

data class Ft8Traffic(val stations: List<Ft8Station>, val links: List<Ft8Link>)

/** What a standard FT8/FT4 message says: who transmits, to whom, and maybe a locator. */
data class Ft8Message(val sender: String, val recipient: String?, val grid: String?) {
    companion object {
        private val GRID = Regex("^[A-R]{2}[0-9]{2}([A-X]{2})?$")
        private val CALL = Regex("^[A-Z0-9/]{3,}$")

        fun parse(text: String): Ft8Message? {
            val t = text.trim().split(Regex("\\s+"))
            if (t.size < 2) return null
            if (t[0] == "CQ") {
                // "CQ CALL GRID" or "CQ DX CALL GRID" (directed CQ)
                val directed = t.size >= 4 && !t[1].any { it.isDigit() }
                val call = t[if (directed) 2 else 1]
                val grid = t.drop(if (directed) 3 else 2).firstOrNull { GRID.matches(it) && it != "RR73" }
                return if (isCall(call)) Ft8Message(call, null, grid) else null
            }
            // "TO FROM info" where info is a locator, a report (-12, R-05) or RRR/RR73/73
            val to = t[0]
            val from = t[1]
            if (!isCall(to) || !isCall(from)) return null
            val grid = t.getOrNull(2)?.takeIf { GRID.matches(it) && it != "RR73" }
            return Ft8Message(from, to, grid)
        }

        private fun isCall(s: String) = CALL.matches(s) && s.any { it.isDigit() } && s.any { it.isLetter() }
    }
}

object Maidenhead {
    /** Center of a 4 or 6 character Maidenhead locator. */
    fun toLatLon(grid: String): GeoPoint? {
        val g = grid.uppercase()
        if (g.length < 4) return null
        val lon0 = (g[0] - 'A') * 20 - 180.0
        val lat0 = (g[1] - 'A') * 10 - 90.0
        if (g[0] !in 'A'..'R' || g[1] !in 'A'..'R' || !g[2].isDigit() || !g[3].isDigit()) return null
        var lon = lon0 + (g[2] - '0') * 2
        var lat = lat0 + (g[3] - '0') * 1
        return if (g.length >= 6 && g[4] in 'A'..'X' && g[5] in 'A'..'X') {
            lon += (g[4] - 'A') * (2.0 / 24) + 1.0 / 24
            lat += (g[5] - 'A') * (1.0 / 24) + 0.5 / 24
            GeoPoint(lat, lon)
        } else {
            GeoPoint(lat + 0.5, lon + 1.0)
        }
    }
}
