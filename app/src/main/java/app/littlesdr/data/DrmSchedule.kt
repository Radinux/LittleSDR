package app.littlesdr.data

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.time.ZonedDateTime

/** One DRM shortwave broadcast of the season's schedule */
data class DrmBroadcast(
    val name: String,
    val hz: Double,
    /** On-air slots in minutes since 00:00 UTC (end may be past midnight: end < start) */
    val slots: List<Pair<Int, Int>>,
    /** Bit 0 = Monday … bit 6 = Sunday */
    val days: Int,
    /** "Occasionally", "Days unconfirmed": shown, but may well be silent */
    val irregular: Boolean,
    val language: String,
    val target: String,
    val site: String,
    /** ITU region of the target: "region_1" = Europe, Africa, Middle East */
    val region: String,
    /** "confirmed", "likely", "uncertain" */
    val confidence: String,
) {
    private fun dayOn(t: ZonedDateTime) = days and (1 shl (t.dayOfWeek.value - 1)) != 0

    fun onAir(t: ZonedDateTime): Boolean {
        val m = t.hour * 60 + t.minute
        return slots.any { (s, e) ->
            if (s <= e) m in s until e && dayOn(t)
            // Across midnight: the evening part belongs to today, the morning part to yesterday's run
            else (m >= s && dayOn(t)) || (m < e && dayOn(t.minusDays(1)))
        }
    }

    /** Minutes until the next start within [withinMin], null if none */
    fun startsIn(t: ZonedDateTime, withinMin: Int = 12 * 60): Int? {
        val m = t.hour * 60 + t.minute
        return slots.mapNotNull { (s, _) ->
            val d = (s - m).mod(24 * 60)
            val day = t.plusMinutes(d.toLong())
            if (d in 1..withinMin && day.dayOfWeek.value.let { days and (1 shl (it - 1)) != 0 }) d else null
        }.minOrNull()
    }

    /** "17:00–18:00 UTC" (first slot) */
    val timeText: String
        get() = slots.firstOrNull()?.let { (s, e) -> "%02d:%02d–%02d:%02d UTC".format(s / 60, s % 60, (e / 60) % 24, e % 60) } ?: ""

    /** "anglais → Europe de l'Ouest" */
    val audience: String get() = listOf(LANG[language] ?: language, TARGET[target] ?: target).filter { it.isNotBlank() }.joinToString(" → ")

    companion object {
        private val LANG = mapOf(
            "English" to "anglais", "French" to "français", "German" to "allemand", "Italian" to "italien",
            "Spanish" to "espagnol", "Chinese" to "chinois", "Korean" to "coréen", "Romanian" to "roumain",
            "Russian" to "russe", "Arabic" to "arabe", "Multi" to "plusieurs langues", "Various" to "divers",
            "Not specified" to "",
        )
        private val TARGET = mapOf(
            "Western Europe" to "Europe de l'Ouest", "central Europe" to "Europe centrale", "Central Europe" to "Europe centrale",
            "Europe" to "Europe", "Italy" to "Italie", "East Africa" to "Afrique de l'Est", "West Africa" to "Afrique de l'Ouest",
            "China" to "Chine", "N China" to "Chine du Nord", "S China" to "Chine du Sud", "N & E China" to "Chine du Nord et de l'Est",
            "East Asia" to "Asie de l'Est", "South Asia" to "Asie du Sud", "China, Japan" to "Chine, Japon",
            "Argentina, Brazil" to "Argentine, Brésil",
        )
        private val DAY_NAMES = listOf("mon", "tue", "wed", "thu", "fri", "sat", "sun")

        /** "Daily", "Tue & Thu", "Sun-Fri", "Mon, Wed": a day mask (every day when unclear) */
        fun parseDays(s: String): Int {
            val t = s.lowercase()
            if (t.isBlank() || "daily" in t || "occasion" in t || "unconfirmed" in t) return 0x7F
            var mask = 0
            for (part in t.split('&', ',', ' ').filter { it.isNotBlank() }) {
                val r = part.split('-').map { p -> DAY_NAMES.indexOfFirst { p.trim().startsWith(it) } }
                if (r.size == 2 && r[0] >= 0 && r[1] >= 0) {
                    var i = r[0]
                    while (true) {
                        mask = mask or (1 shl i)
                        if (i == r[1]) break
                        i = (i + 1) % 7
                    }
                } else if (r.size == 1 && r[0] >= 0) mask = mask or (1 shl r[0])
            }
            return if (mask == 0) 0x7F else mask
        }

        /** "0800-1200 2200-0100" */
        fun parseSlots(s: String): List<Pair<Int, Int>> = Regex("(\\d{2})(\\d{2})-(\\d{2})(\\d{2})").findAll(s).map { m ->
            val (h1, m1, h2, m2) = m.destructured
            (h1.toInt() * 60 + m1.toInt()) to (h2.toInt() * 60 + m2.toInt()).coerceAtMost(24 * 60)
        }.toList()

        fun parse(json: String): List<DrmBroadcast> = runCatching {
            val o = Json.parseToJsonElement(json).jsonObject
            (o["records"] as JsonArray).mapNotNull { rec ->
                val r = rec.jsonObject
                val v = r["values"]!!.jsonArray.map { it.jsonPrimitive.content }
                val khz = v.getOrNull(2)?.trim()?.toDoubleOrNull() ?: return@mapNotNull null
                val slots = parseSlots(v[0])
                if (slots.isEmpty()) return@mapNotNull null
                val days = v.getOrElse(3) { "" }
                DrmBroadcast(
                    name = v.getOrElse(1) { "" }.trim(),
                    hz = khz * 1_000,
                    slots = slots,
                    days = parseDays(days),
                    irregular = days.contains("occasion", true) || days.contains("unconfirmed", true),
                    language = v.getOrElse(4) { "" }.trim(),
                    target = v.getOrElse(5) { "" }.trim(),
                    site = v.getOrElse(6) { "" }.trim(),
                    region = (r["region"]?.jsonPrimitive?.content) ?: "",
                    confidence = (r["confidence"]?.jsonPrimitive?.content) ?: "",
                )
            }
        }.getOrDefault(emptyList())
    }
}

/**
 * The DRM Consortium's shortwave schedule (drm.org): bundled with the app for the season it was
 * built in, refreshed from the site once a week (its page carries the schedule as JSON). Only
 * the page is fetched, nothing is sent.
 */
object DrmSchedule {
    private const val URL_PAGE = "https://www.drm.org/drm-in-the-world/broadcast-schedule/"
    private const val MAX_AGE_MS = 7L * 24 * 3600 * 1000

    private fun cache(ctx: Context) = File(ctx.filesDir, "drm_schedule.json")

    /** The schedule: the downloaded one if any, else the bundled one */
    suspend fun load(ctx: Context): List<DrmBroadcast> = withContext(Dispatchers.IO) {
        val f = cache(ctx)
        val text = if (f.exists()) f.readText() else ctx.assets.open("drm_schedule.json").use { it.readBytes().decodeToString() }
        DrmBroadcast.parse(text).ifEmpty { DrmBroadcast.parse(ctx.assets.open("drm_schedule.json").use { it.readBytes().decodeToString() }) }
    }

    /** Downloads the current schedule when the copy is a week old; true if it changed */
    suspend fun refresh(ctx: Context): Boolean = withContext(Dispatchers.IO) {
        val f = cache(ctx)
        if (f.exists() && System.currentTimeMillis() - f.lastModified() < MAX_AGE_MS) return@withContext false
        runCatching {
            val c = URL(URL_PAGE).openConnection() as HttpURLConnection
            c.connectTimeout = 8_000
            c.readTimeout = 15_000
            c.setRequestProperty("User-Agent", "Mozilla/5.0 (LittleSDR)")
            val page = try { c.inputStream.use { it.readBytes().decodeToString() } } finally { c.disconnect() }
            val raw = Regex("<script id=\"drm-sw-data\" type=\"application/json\">(.*?)</script>", RegexOption.DOT_MATCHES_ALL).find(page)?.groupValues?.get(1)
                ?: return@runCatching false
            val o = Json.parseToJsonElement(raw).jsonObject
            // Same record format as the bundled file (tools/make_drm_schedule.py)
            val slim = JsonObject(mapOf("season" to (o["season"] ?: Json.parseToJsonElement("\"\"")), "records" to o["records"]!!))
            val text = slim.toString()
            if (DrmBroadcast.parse(text).isEmpty()) return@runCatching false
            f.writeText(text)
            true
        }.getOrDefault(false)
    }
}
