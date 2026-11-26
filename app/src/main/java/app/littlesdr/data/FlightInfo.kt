package app.littlesdr.data

import android.graphics.BitmapFactory
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import kotlin.math.asin
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt

/** An airport of a route */
data class Airport(val name: String, val city: String, val iata: String, val icao: String, val country: String, val lat: Double, val lon: Double) {
    /** "MXP", else "LIMC" */
    val code: String get() = iata.ifBlank { icao }
    /** "Milan", else the airport's name */
    val place: String get() = city.ifBlank { name }
}

/** Where a flight number flies from and to, and who flies it */
data class FlightRoute(val origin: Airport, val destination: Airport, val airline: String) {
    /**
     * Callsign databases are filled in by volunteers and get out of date: the route is only
     * believed when the aircraft is not far off the way between the two airports.
     */
    fun plausible(lat: Double?, lon: Double?): Boolean? {
        if (lat == null || lon == null) return null
        val direct = km(origin.lat, origin.lon, destination.lat, destination.lon)
        val via = km(origin.lat, origin.lon, lat, lon) + km(lat, lon, destination.lat, destination.lon)
        return via <= 1.25 * direct + 150
    }
}

/** The airframe behind an ICAO address */
data class AircraftDetails(val type: String, val manufacturer: String, val registration: String, val owner: String, val photoUrl: String?)

private fun km(la1: Double, lo1: Double, la2: Double, lo2: Double): Double {
    val p1 = Math.toRadians(la1)
    val p2 = Math.toRadians(la2)
    val h = sin((p2 - p1) / 2).pow(2) + cos(p1) * cos(p2) * sin(Math.toRadians(lo2 - lo1) / 2).pow(2)
    return 2 * 6371 * asin(sqrt(h))
}

/**
 * Flight routes and airframes from adsbdb.com (open, no key). Only the flight number or the
 * ICAO address leave the phone: public data the aircraft broadcast anyway. Answers are kept
 * for the session, misses too (a flight number unknown now stays unknown).
 */
object FlightInfo {
    private const val API = "https://api.adsbdb.com/v0"
    private val json = Json { ignoreUnknownKeys = true }
    private val routes = HashMap<String, FlightRoute?>()
    private val airframes = HashMap<String, AircraftDetails?>()
    private val photos = HashMap<String, ImageBitmap?>()
    private val lock = Mutex()
    /** One request at a time: a sky full of aircraft must not flood the service */
    private val net = Mutex()

    fun knownRoute(callsign: String): FlightRoute? = routes[callsign]
    fun askedRoute(callsign: String): Boolean = routes.containsKey(callsign)

    suspend fun route(callsign: String): FlightRoute? {
        val cs = callsign.trim().uppercase()
        if (cs.length < 3) return null
        lock.withLock { if (routes.containsKey(cs)) return routes[cs] }
        val obj = fetch("$API/callsign/${URLEncoder.encode(cs, "UTF-8")}") ?: return null // network: ask again later
        val r = runCatching {
            val f = obj["flightroute"]!!.jsonObject
            FlightRoute(airport(f["origin"]!!.jsonObject), airport(f["destination"]!!.jsonObject), f["airline"]?.let { s(it.jsonObject, "name") } ?: "")
        }.getOrNull()
        lock.withLock { routes[cs] = r }
        return r
    }

    suspend fun aircraft(icao: String): AircraftDetails? {
        val hex = icao.trim().uppercase()
        lock.withLock { if (airframes.containsKey(hex)) return airframes[hex] }
        val obj = fetch("$API/aircraft/$hex") ?: return null
        val a = runCatching {
            val o = obj["aircraft"]!!.jsonObject
            AircraftDetails(s(o, "type"), s(o, "manufacturer"), s(o, "registration"), s(o, "registered_owner"),
                s(o, "url_photo").ifBlank { s(o, "url_photo_thumbnail") }.takeIf { it.startsWith("http") })
        }.getOrNull()
        lock.withLock { airframes[hex] = a }
        return a
    }

    suspend fun photo(url: String): ImageBitmap? {
        lock.withLock { if (photos.containsKey(url)) return photos[url] }
        val bmp = withContext(Dispatchers.IO) {
            net.withLock { runCatching { get(url, 1_000_000) }.getOrNull() }?.let { BitmapFactory.decodeByteArray(it, 0, it.size)?.asImageBitmap() }
        }
        lock.withLock { photos[url] = bmp }
        return bmp
    }

    private fun s(o: JsonObject, k: String): String = runCatching { o[k]?.jsonPrimitive?.content }.getOrNull()?.takeIf { it != "null" } ?: ""
    private fun d(o: JsonObject, k: String): Double = runCatching { o[k]?.jsonPrimitive?.content?.toDouble() }.getOrNull() ?: 0.0

    private fun airport(o: JsonObject) = Airport(s(o, "name"), s(o, "municipality"), s(o, "iata_code"), s(o, "icao_code"), s(o, "country_name"), d(o, "latitude"), d(o, "longitude"))

    /** The "response" object; null when the service cannot be reached (not cached) */
    private suspend fun fetch(url: String): JsonObject? = withContext(Dispatchers.IO) {
        net.withLock {
            val body = runCatching { get(url, 64_000) }.getOrNull() ?: return@withContext null
            // Not JSON (an error page): no answer, asked again later
            val top = runCatching { json.parseToJsonElement(String(body)).jsonObject }.getOrNull() ?: return@withContext null
            // "unknown callsign" / "unknown aircraft": a plain string, an answer all the same
            (top["response"] as? JsonObject) ?: JsonObject(emptyMap())
        }
    }

    private fun get(url: String, max: Int): ByteArray? {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 6_000
        c.readTimeout = 8_000
        c.setRequestProperty("User-Agent", "LittleSDR")
        return try {
            // 404 for an unknown callsign still carries the JSON answer
            val s = if (c.responseCode in 200..299) c.inputStream else c.errorStream ?: return null
            s.use { inp ->
                val out = java.io.ByteArrayOutputStream()
                val buf = ByteArray(16_384)
                while (out.size() < max) {
                    val n = inp.read(buf)
                    if (n < 0) break
                    out.write(buf, 0, n)
                }
                out.toByteArray()
            }
        } finally {
            c.disconnect()
        }
    }
}
