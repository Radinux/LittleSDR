package app.littlesdr.data

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlin.math.asin
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt

/** One public KiwiSDR receiver from the kiwisdr.com list. */
data class KiwiServer(
    val name: String,
    val location: String,
    val lat: Double?,
    val lon: Double?,
    val users: Int,
    val usersMax: Int,
    /** "host:port" for the WebSocket connection */
    val hostPort: String,
    val antenna: String,
    /** "0-30000000" style range, Hz */
    val bandsText: String,
) {
    val free: Boolean get() = users < usersMax

    fun distanceKm(fromLat: Double, fromLon: Double): Double? {
        val la = lat ?: return null
        val lo = lon ?: return null
        val r = Math.toRadians(1.0)
        val h = sin((la - fromLat) * r / 2).pow(2) + cos(fromLat * r) * cos(la * r) * sin((lo - fromLon) * r / 2).pow(2)
        return 2 * 6371.0 * asin(sqrt(h.coerceIn(0.0, 1.0)))
    }
}

/**
 * Public KiwiSDR list (same source as SDR++Brown's map: rx.linkfanel.net, generated from
 * kiwisdr.com/public), cached for a day.
 */
class KiwiDirectory(context: Context) {
    private val cache = File(context.filesDir, "kiwisdr_com.js")
    @Volatile var servers: List<KiwiServer> = emptyList()
        private set

    fun lastUpdate(): Long = if (cache.exists()) cache.lastModified() else 0L

    suspend fun load(force: Boolean = false): List<KiwiServer> = withContext(Dispatchers.IO) {
        val stale = !cache.exists() || System.currentTimeMillis() - cache.lastModified() > 86_400_000L
        if (force || stale) download()
        if (cache.exists()) servers = runCatching { parse(cache.readText()) }.getOrDefault(servers)
        servers
    }

    private fun download(): Boolean = runCatching {
        val conn = URL(LIST_URL).openConnection() as HttpURLConnection
        conn.connectTimeout = 10_000
        conn.readTimeout = 30_000
        try {
            if (conn.responseCode != 200) return false
            val tmp = File(cache.parentFile, "kiwisdr_com.tmp")
            conn.inputStream.use { input -> tmp.outputStream().use { input.copyTo(it) } }
            tmp.renameTo(cache)
        } finally {
            conn.disconnect()
        }
    }.getOrDefault(false)

    companion object {
        private const val LIST_URL = "http://rx.linkfanel.net/kiwisdr_com.js"
        private val json = Json { ignoreUnknownKeys = true; isLenient = true }
        private val GPS = Regex("""\(\s*(-?[\d.]+)\s*,\s*(-?[\d.]+)\s*\)""")
        private val TRAILING_COMMA = Regex(""",\s*([\]}])""")

        /** "var kiwisdr_com = [ {...}, ... ];" -> active, online servers with a usable address. */
        fun parse(js: String): List<KiwiServer> {
            val start = js.indexOf('[')
            val end = js.lastIndexOf(']')
            if (start < 0 || end <= start) return emptyList()
            val body = TRAILING_COMMA.replace(js.substring(start, end + 1), "$1")
            val arr = json.parseToJsonElement(body) as? JsonArray ?: return emptyList()
            return arr.mapNotNull { el ->
                val o = el as? JsonObject ?: return@mapNotNull null
                fun str(k: String) = o[k]?.jsonPrimitive?.content.orEmpty()
                if (str("offline") != "no" || str("status") != "active") return@mapNotNull null
                val hostPort = hostPort(str("url")) ?: return@mapNotNull null
                val gps = GPS.find(str("gps"))
                KiwiServer(
                    name = str("name").trim(),
                    location = str("loc").trim(),
                    lat = gps?.groupValues?.get(1)?.toDoubleOrNull(),
                    lon = gps?.groupValues?.get(2)?.toDoubleOrNull(),
                    users = str("users").toIntOrNull() ?: 0,
                    usersMax = str("users_max").toIntOrNull() ?: 0,
                    hostPort = hostPort,
                    antenna = str("antenna").trim(),
                    bandsText = str("bands"),
                )
            }
        }

        /** "http://host:8073/" -> "host:8073" (port 80 when absent). Plain http only. */
        fun hostPort(url: String): String? {
            val u = url.trim().removeSuffix("/")
            if (!u.startsWith("http://")) return null
            val hp = u.removePrefix("http://").substringBefore('/')
            if (hp.isBlank()) return null
            return if (':' in hp) hp else "$hp:80"
        }
    }
}
