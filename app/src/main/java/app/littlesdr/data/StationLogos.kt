package app.littlesdr.data

import android.content.Context
import android.graphics.BitmapFactory
import android.net.ConnectivityManager
import android.telephony.TelephonyManager
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.MessageDigest
import java.util.Locale

/**
 * Station logos from radio-browser.info (open community directory), looked up by the station
 * name and the country of the mobile network. Only those two leave the phone. Kept on disk:
 * each logo is downloaded once; a station without a logo is asked again after a week.
 *
 * Off on mobile data unless allowed (settings): Wi-Fi / unmetered networks only by default.
 */
object StationLogos {
    private const val UA = "LittleSDR"
    private val SERVERS = listOf("de1.api.radio-browser.info", "nl1.api.radio-browser.info", "at1.api.radio-browser.info")
    private const val MISS_TTL_MS = 7L * 24 * 3600 * 1000
    private const val MAX_BYTES = 512 * 1024

    private lateinit var app: Context
    private lateinit var dir: File
    private val memory = HashMap<String, ImageBitmap?>()
    private val lock = Mutex()
    private val json = Json { ignoreUnknownKeys = true }

    private val _enabled = MutableStateFlow(true)
    /** Fetch logos from the internet at all */
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()
    private val _mobileData = MutableStateFlow(false)
    /** Also over mobile data (metered networks) */
    val mobileData: StateFlow<Boolean> = _mobileData.asStateFlow()

    fun init(context: Context) {
        if (::app.isInitialized) return
        app = context.applicationContext
        dir = File(app.cacheDir, "logos").apply { mkdirs() }
        val p = app.getSharedPreferences("logos", Context.MODE_PRIVATE)
        _enabled.value = p.getBoolean("enabled", true)
        _mobileData.value = p.getBoolean("mobile", false)
    }

    fun setEnabled(on: Boolean) {
        _enabled.value = on
        app.getSharedPreferences("logos", Context.MODE_PRIVATE).edit().putBoolean("enabled", on).apply()
    }

    fun setMobileData(on: Boolean) {
        _mobileData.value = on
        app.getSharedPreferences("logos", Context.MODE_PRIVATE).edit().putBoolean("mobile", on).apply()
    }

    private fun country(): String {
        val tm = app.getSystemService(TelephonyManager::class.java)
        return (tm?.networkCountryIso?.takeIf { it.isNotBlank() } ?: Locale.getDefault().country).uppercase(Locale.ROOT)
    }

    /** Allowed to download now: logos on, and not on mobile data unless allowed */
    private fun mayDownload(): Boolean {
        if (!_enabled.value) return false
        val cm = app.getSystemService(ConnectivityManager::class.java) ?: return false
        if (cm.activeNetwork == null) return false
        return !cm.isActiveNetworkMetered || _mobileData.value
    }

    private fun norm(s: String) = s.lowercase(Locale.ROOT).filter { it.isLetterOrDigit() }

    private fun key(name: String, cc: String): String =
        MessageDigest.getInstance("SHA-1").digest("${norm(name)}|$cc".toByteArray())
            .joinToString("") { "%02x".format(it) }

    /** The logo of a station, or null (none known, not allowed to download, offline) */
    suspend fun get(name: String): ImageBitmap? {
        if (!::app.isInitialized || norm(name).length < 2) return null
        val cc = country()
        val k = key(name, cc)
        lock.withLock { if (memory.containsKey(k)) return memory[k] }
        val bmp = withContext(Dispatchers.IO) { load(name, cc, k) }
        // A miss caused by the network rules is not remembered: retried later on Wi-Fi
        if (bmp != null || File(dir, "$k.none").exists()) lock.withLock { memory[k] = bmp }
        return bmp
    }

    private fun load(name: String, cc: String, k: String): ImageBitmap? {
        val img = File(dir, "$k.img")
        if (img.exists()) return decode(img.readBytes())
        val none = File(dir, "$k.none")
        if (none.exists() && System.currentTimeMillis() - none.lastModified() < MISS_TTL_MS) return null
        if (!mayDownload()) return null
        val url = find(name, cc) ?: find(name, null)
        val bytes = url?.let { download(it) }
        val bmp = bytes?.let { decode(it) }
        if (bmp != null) img.writeBytes(bytes) else none.writeText("")
        return bmp
    }

    /** Best favicon URL for a name: exact name first, then a name starting with it; country first */
    private fun find(name: String, cc: String?): String? {
        val q = URLEncoder.encode(name.trim(), "UTF-8")
        val path = "/json/stations/search?name=$q&limit=20&hidebroken=true&order=clickcount&reverse=true" +
            (cc?.let { "&countrycode=$it" } ?: "")
        val body = SERVERS.firstNotNullOfOrNull { host -> runCatching { http("https://$host$path") }.getOrNull() } ?: return null
        val list = runCatching { json.parseToJsonElement(String(body)) as JsonArray }.getOrNull() ?: return null
        val want = norm(name)
        val candidates = list.mapNotNull { e ->
            val o = e.jsonObject
            val n = o["name"]?.jsonPrimitive?.content ?: return@mapNotNull null
            val fav = o["favicon"]?.jsonPrimitive?.content?.takeIf { it.startsWith("http") } ?: return@mapNotNull null
            norm(n) to fav
        }
        return candidates.firstOrNull { it.first == want }?.second
            ?: candidates.firstOrNull { it.first.startsWith(want) }?.second
    }

    private fun http(url: String): ByteArray? {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 6_000
        c.readTimeout = 8_000
        c.instanceFollowRedirects = true
        c.setRequestProperty("User-Agent", UA)
        return try {
            if (c.responseCode !in 200..299) return null
            c.inputStream.use { s ->
                val out = java.io.ByteArrayOutputStream()
                val buf = ByteArray(16_384)
                while (true) {
                    val n = s.read(buf)
                    if (n < 0) break
                    out.write(buf, 0, n)
                    if (out.size() > MAX_BYTES) return null
                }
                out.toByteArray()
            }
        } finally {
            c.disconnect()
        }
    }

    private fun download(url: String): ByteArray? = runCatching { http(url) }.getOrNull()

    private fun decode(b: ByteArray): ImageBitmap? = BitmapFactory.decodeByteArray(b, 0, b.size)?.asImageBitmap()
}
