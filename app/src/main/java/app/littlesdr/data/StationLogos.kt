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
    /** Written in the "no logo" files: bump it when the search improves, so misses are retried */
    private const val MISS_VERSION = "2"

    private lateinit var app: Context
    private lateinit var dir: File
    private val memory = HashMap<String, ImageBitmap?>()
    private val lock = Mutex()
    private val json = Json { ignoreUnknownKeys = true }

    private val _enabled = MutableStateFlow(true)
    /** Fetch logos from the internet at all */
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()
    /** Bumps when a logo appears without a request (taken from the multiplex) */
    private val _revision = MutableStateFlow(0)
    val revision: StateFlow<Int> = _revision.asStateFlow()

    /** Per station: slides seen (content hash -> programme texts it was shown under) */
    private val slidesSeen = HashMap<String, HashMap<String, MutableList<String>>>()

    /**
     * A slideshow picture from the multiplex: kept as the station's logo when it really is
     * one, never an album cover. Either its name says "logo", or the same picture keeps
     * coming back across different songs (3 times, under 2 programme texts at least): a
     * cover goes with one song, a logo with all of them. Used only when the directory
     * online has no logo for the station.
     */
    suspend fun offerSlide(station: String?, bytes: ByteArray, name: String, text: String) {
        if (!::app.isInitialized || station.isNullOrBlank() || bytes.isEmpty()) return
        val hash = MessageDigest.getInstance("SHA-1").digest(bytes).joinToString("") { "%02x".format(it) }
        val isLogo = lock.withLock {
            val seen = slidesSeen.getOrPut(station) { HashMap() }.getOrPut(hash) { mutableListOf() }
            seen += text.trim()
            name.contains("logo", ignoreCase = true) || (seen.size >= 3 && seen.distinct().size >= 2)
        }
        if (!isLogo) return
        val k = key(station, country())
        val mux = File(dir, "$k.mux")
        if (File(dir, "$k.img").exists() || (mux.exists() && mux.length() == bytes.size.toLong())) return
        val bmp = withContext(Dispatchers.IO) {
            decode(bytes)?.also { mux.writeBytes(bytes) }
        } ?: return
        lock.withLock {
            // The online one wins when there is one; this fills the gap otherwise
            if (!File(dir, "$k.img").exists()) memory[k] = bmp
        }
        _revision.value++
    }

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
        if (bmp != null || File(dir, "$k.none").exists()) lock.withLock { if (memory[k] == null) memory[k] = bmp }
        return bmp
    }

    private fun load(name: String, cc: String, k: String): ImageBitmap? {
        val img = File(dir, "$k.img")
        if (img.exists()) return decode(img.readBytes())
        // The logo the station itself sent on the multiplex, when the directory has none
        val mux = File(dir, "$k.mux").takeIf { it.exists() }?.let { decode(it.readBytes()) }
        val none = File(dir, "$k.none")
        // A miss found by the current search only ("2"): those of the old search (empty file)
        // are looked up again
        if (none.exists() && none.readText() == MISS_VERSION && System.currentTimeMillis() - none.lastModified() < MISS_TTL_MS) return mux
        if (!mayDownload()) return mux
        val bytes = lookup(name, cc)
        val bmp = bytes?.let { decode(it) }
        if (bmp != null) img.writeBytes(bytes) else none.writeText(MISS_VERSION)
        return bmp ?: mux
    }

    /** Words that are not part of a station's name: "FUN RADIO DAB+", "LATINA dab+" */
    private val NOISE = setOf("dab", "dabplus", "hd", "digital", "numerique", "numérique")

    private fun clean(name: String): String =
        name.trim().split(Regex("\\s+")).filter { w -> norm(w).let { it.isNotEmpty() && it !in NOISE } }.joinToString(" ")

    private class Station(val name: String, val favicon: String?, val homepage: String?, val clicks: Long)

    /**
     * A logo for a station name, as bytes that decode:
     * - the directory's entries named like it (exact name first, then names starting with
     *   it; the country of the network first, then anywhere), most listened first;
     * - each candidate's favicon, else the icons of its website (apple-touch-icon first);
     *   several are tried, as many favicons are broken links or SVG;
     * - the brand last ("NOSTALGIE 60/70" -> Nostalgie), its exact name only.
     * Small icons (under 64 px) are kept only if nothing better turns up.
     */
    private fun lookup(name: String, cc: String): ByteArray? {
        val c = clean(name).ifBlank { name.trim() }
        val words = c.split(' ')
        val brand = if (words[0].equals("radio", ignoreCase = true) && words.size > 1) words.take(2).joinToString(" ") else words[0]
        val stages = buildList {
            add(Triple(c, cc, true))
            add(Triple(c, null, true))
            if (norm(brand) != norm(c) && norm(brand).length >= 3) {
                add(Triple(brand, cc, false))
                add(Triple(brand, null, false))
            }
        }
        val tried = HashSet<String>()
        var small: ByteArray? = null
        var fetches = 0
        fun attempt(url: String): ByteArray? {
            if (fetches >= 16 || !tried.add(url)) return null
            fetches++
            val b = download(url) ?: return null
            val px = pixels(b) ?: return null
            if (px >= 64) return b
            if (small == null) small = b
            return null
        }
        for ((q, ccq, byName) in stages) {
            val want = norm(q)
            val found = search(q, ccq)
                .mapNotNull { st ->
                    val n = norm(st.name)
                    val score = when {
                        n == want -> 2
                        byName && n.startsWith(want) -> 1
                        else -> 0
                    }
                    if (score > 0) score to st else null
                }
                .sortedWith(compareByDescending<Pair<Int, Station>> { it.first }.thenByDescending { it.second.clicks })
                .take(6)
                .map { it.second }
            for (st in found) st.favicon?.let { attempt(it) }?.let { return it }
            // No usable favicon: the stations' own websites
            for (home in found.mapNotNull { it.homepage }.distinct().take(2)) {
                for (u in siteIcons(home).take(4)) attempt(u)?.let { return it }
            }
        }
        return small
    }

    /** Directory entries for a name (the directory matches names containing it) */
    private fun search(q: String, cc: String?): List<Station> {
        val path = "/json/stations/search?name=${URLEncoder.encode(q, "UTF-8")}&limit=30&hidebroken=true&order=clickcount&reverse=true" +
            (cc?.let { "&countrycode=$it" } ?: "")
        val body = SERVERS.firstNotNullOfOrNull { host -> runCatching { http("https://$host$path") }.getOrNull() } ?: return emptyList()
        val list = runCatching { json.parseToJsonElement(String(body)) as JsonArray }.getOrNull() ?: return emptyList()
        return list.mapNotNull { e ->
            val o = e.jsonObject
            val n = o["name"]?.jsonPrimitive?.content ?: return@mapNotNull null
            fun url(k: String) = o[k]?.jsonPrimitive?.content?.trim()?.takeIf { it.startsWith("http") }
            Station(n, url("favicon"), url("homepage"), o["clickcount"]?.jsonPrimitive?.content?.toLongOrNull() ?: 0)
        }
    }

    /** Icons a website declares (apple-touch-icon, large icons, other icons), then the usual path */
    private fun siteIcons(home: String): List<String> {
        val (html, base) = runCatching { httpWithUrl(home, 400_000, truncate = true) }.getOrNull() ?: return emptyList()
        val page = String(html)
        val found = ArrayList<Pair<Int, String>>()
        for (m in Regex("<link\\b[^>]*>", RegexOption.IGNORE_CASE).findAll(page)) {
            val tag = m.value
            val rel = Regex("rel\\s*=\\s*[\"']([^\"']+)", RegexOption.IGNORE_CASE).find(tag)?.groupValues?.get(1)?.lowercase(Locale.ROOT) ?: continue
            val href = Regex("href\\s*=\\s*[\"']([^\"']+)", RegexOption.IGNORE_CASE).find(tag)?.groupValues?.get(1) ?: continue
            val size = Regex("sizes\\s*=\\s*[\"'](\\d+)", RegexOption.IGNORE_CASE).find(tag)?.groupValues?.get(1)?.toIntOrNull() ?: 0
            val rank = when {
                "apple-touch-icon" in rel -> 0
                "icon" in rel && size >= 96 -> 1
                "icon" in rel -> 2
                else -> continue
            }
            found += rank to href
        }
        found += 3 to "/apple-touch-icon.png"
        return found.sortedBy { it.first }.mapNotNull { runCatching { URL(base, it.second.replace("&amp;", "&")).toString() }.getOrNull() }.distinct()
    }

    /** Smallest side in pixels if the bytes decode as a picture, else null (SVG, HTML...) */
    private fun pixels(b: ByteArray): Int? {
        val o = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(b, 0, b.size, o)
        return if (o.outWidth > 0 && o.outHeight > 0) minOf(o.outWidth, o.outHeight) else null
    }

    private fun http(url: String): ByteArray? = httpWithUrl(url, MAX_BYTES)?.first

    /**
     * GET with redirects followed by hand (HttpURLConnection does not go from http to https,
     * which most station sites do); the body and the final URL
     */
    private fun httpWithUrl(start: String, maxBytes: Int, truncate: Boolean = false): Pair<ByteArray, URL>? {
        var url = URL(start)
        repeat(5) {
            val c = url.openConnection() as HttpURLConnection
            c.connectTimeout = 6_000
            c.readTimeout = 8_000
            c.instanceFollowRedirects = false
            c.setRequestProperty("User-Agent", UA)
            try {
                val code = c.responseCode
                if (code in 300..399) {
                    url = URL(url, c.getHeaderField("Location") ?: return null)
                    return@repeat
                }
                if (code !in 200..299) return null
                c.inputStream.use { s ->
                    val out = java.io.ByteArrayOutputStream()
                    val buf = ByteArray(16_384)
                    while (true) {
                        val n = s.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        // A web page: its head is enough. A picture that big: refused
                        if (out.size() > maxBytes) return if (truncate) out.toByteArray() to url else null
                    }
                    return out.toByteArray() to url
                }
            } finally {
                c.disconnect()
            }
        }
        return null
    }

    private fun download(url: String): ByteArray? = runCatching { http(url) }.getOrNull()

    private fun decode(b: ByteArray): ImageBitmap? = BitmapFactory.decodeByteArray(b, 0, b.size)?.asImageBitmap()
}
