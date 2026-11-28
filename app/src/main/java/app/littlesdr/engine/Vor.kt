package app.littlesdr.engine

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/** VOR beacons: 108 - 117.95 MHz, 50 kHz apart */
val VOR_BAND = 108_000_000.0..117_950_000.0
const val VOR_STEP = 50_000.0

/** What the engine's VOR receiver reads */
@Serializable
data class VorState(
    val active: Boolean = false,
    /** Magnetic radial from the beacon (deg), when a VOR is received */
    val radial: Double? = null,
    /** 30 Hz variable modulation depth (nominally 0.3) */
    val depth: Double = 0.0,
    /** Reference deviation (nominally 480 Hz) */
    val deviation: Double = 0.0,
    /** 1 = the radial holds still */
    val steadiness: Double = 0.0,
    /** The ident, once heard twice */
    val ident: String = "",
    /** The Morse heard lately */
    val morse: String = "",
) {
    /** Course to fly to the beacon (magnetic) */
    val toBeacon: Double? get() = radial?.let { (it + 180.0) % 360.0 }

    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): VorState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(VorState())
    }
}

/** A VOR beacon of the list (OurAirports, tools/make_vor_list.py) */
data class VorStation(
    val ident: String,
    val name: String,
    val hz: Double,
    val lat: Double,
    val lon: Double,
    val country: String,
    /** The beacon's alignment: magnetic north = true north + this (deg, east positive) */
    val declination: Double,
) {
    val position: GeoPoint get() = GeoPoint(lat, lon)
    val mhz: String get() = "%.2f MHz".format(java.util.Locale.FRANCE, hz / 1e6)
}

object VorStations {
    @Volatile private var cache: List<VorStation>? = null

    suspend fun load(ctx: Context): List<VorStation> = cache ?: withContext(Dispatchers.IO) {
        val text = ctx.assets.open("vor.json").use { it.readBytes().decodeToString() }
        runCatching {
            Json.parseToJsonElement(text).jsonArray.map { e ->
                val v = e.jsonArray.map { it.jsonPrimitive.content }
                VorStation(v[0], v[1], v[2].toDouble() * 1_000, v[3].toDouble(), v[4].toDouble(), v[5], v[6].toDouble())
            }
        }.getOrDefault(emptyList()).also { cache = it }
    }

    /** The beacon on this frequency: the one whose ident was heard, else the nearest */
    fun at(list: List<VorStation>, hz: Double, ident: String, from: GeoPoint?): VorStation? {
        val here = list.filter { kotlin.math.abs(it.hz - hz) < 1_000 }
        return here.firstOrNull { ident.isNotBlank() && it.ident.equals(ident, ignoreCase = true) && (from == null || distanceKm(from, it.position) < 500) }
            ?: from?.let { p -> here.minByOrNull { distanceKm(p, it.position) }?.takeIf { distanceKm(p, it.position) < 400 } }
    }

    /** The nearest beacons (a VOR carries ~200 km at altitude, much less on the ground) */
    fun nearest(list: List<VorStation>, from: GeoPoint, n: Int = 12): List<VorStation> =
        list.sortedBy { distanceKm(from, it.position) }.take(n)
}

/** Great-circle distance (km) */
fun distanceKm(a: GeoPoint, b: GeoPoint): Double {
    val r = Math.PI / 180
    val dLat = (b.lat - a.lat) * r
    val dLon = (b.lon - a.lon) * r
    val h = sin(dLat / 2) * sin(dLat / 2) + cos(a.lat * r) * cos(b.lat * r) * sin(dLon / 2) * sin(dLon / 2)
    return 6371.0 * 2 * atan2(sqrt(h), sqrt(1 - h))
}

/** Initial true bearing (deg) from a to b */
fun bearingDeg(a: GeoPoint, b: GeoPoint): Double {
    val r = Math.PI / 180
    val y = sin((b.lon - a.lon) * r) * cos(b.lat * r)
    val x = cos(a.lat * r) * sin(b.lat * r) - sin(a.lat * r) * cos(b.lat * r) * cos((b.lon - a.lon) * r)
    return (Math.toDegrees(atan2(y, x)) + 360) % 360
}

/** "NE", "SO"... for a bearing (French) */
fun compassPoint(deg: Double): String =
    listOf("N", "NE", "E", "SE", "S", "SO", "O", "NO")[(((deg % 360) + 360) % 360 / 45).roundToInt() % 8]
