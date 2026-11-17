package app.littlesdr.data

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.time.DayOfWeek
import java.time.LocalDate
import java.time.ZoneOffset
import java.time.ZonedDateTime
import java.time.temporal.TemporalAdjusters
import kotlin.math.abs

/** One scheduled broadcast from the EiBi shortwave database (eibispace.de). */
data class EiBiEntry(
    val khz: Double,
    /** Minutes since 00:00 UTC */
    val start: Int,
    val end: Int,
    /** Bit 0 = Monday … bit 6 = Sunday; 0x7F = every day */
    val days: Int,
    val country: String,
    val station: String,
    val language: String,
    val target: String,
) {
    val timeText: String get() = "%02d%02d-%02d%02d".format(start / 60, start % 60, end / 60, end % 60)

    fun activeAt(t: ZonedDateTime): Boolean {
        val dayBit = 1 shl (t.dayOfWeek.value - 1)
        if (days and dayBit == 0) return false
        val m = t.hour * 60 + t.minute
        return if (start <= end) m in start until end else m >= start || m < end // crosses midnight
    }

    /** Readable language: common EiBi codes, "-XX" entries are transmission types. */
    val languageText: String
        get() = when (language) {
            "" -> ""
            "-CW" -> "Morse"
            "-TS" -> "Signal horaire"
            "-HF" -> "Données"
            else -> LANGUAGES[language] ?: language
        }

    companion object {
        private val LANGUAGES = mapOf(
            "M" to "mandarin", "TB" to "tibétain", "E" to "anglais", "F" to "français", "D" to "allemand", "S" to "espagnol", "R" to "russe",
            "A" to "arabe", "C" to "chinois", "P" to "portugais", "I" to "italien", "J" to "japonais",
            "K" to "coréen", "TU" to "turc", "HI" to "hindi", "VN" to "vietnamien",
        )
    }
}

/**
 * EiBi schedule: downloaded once per season (refreshed weekly), parsed into a frequency-sorted list.
 * Data © Eike Bierwirth, eibispace.de — free for non-commercial use.
 */
class EiBiRepository(private val context: Context) {
    private val dir = File(context.filesDir, "eibi")
    @Volatile private var entries: List<EiBiEntry> = emptyList()
    @Volatile var loadedSeason: String = ""
        private set
    val count: Int get() = entries.size

    /** Season code: "a26" (last Sunday of March → last Sunday of October) or "b26" (winter). */
    fun currentSeason(today: LocalDate = LocalDate.now(ZoneOffset.UTC)): String {
        fun lastSunday(month: Int, year: Int) =
            LocalDate.of(year, month, 1).with(TemporalAdjusters.lastInMonth(DayOfWeek.SUNDAY))
        val y = today.year
        return when {
            !today.isBefore(lastSunday(3, y)) && today.isBefore(lastSunday(10, y)) -> "a${y % 100}"
            today.isBefore(lastSunday(3, y)) -> "b${(y - 1) % 100}" // winter that started last year
            else -> "b${y % 100}"
        }
    }

    private fun previousSeason(s: String): String {
        val yy = s.drop(1).toInt()
        return if (s[0] == 'b') "a$yy" else "b${(yy + 99) % 100}"
    }

    private fun file(season: String) = File(dir, "sked-$season.csv")

    /** Cached file for the current season (or the previous one), if any. */
    private fun cached(): Pair<String, File>? {
        val s = currentSeason()
        return listOf(s, previousSeason(s)).map { it to file(it) }.firstOrNull { it.second.exists() }
    }

    fun lastUpdate(): Long = cached()?.second?.lastModified() ?: 0L

    /** Loads the cache, downloading first when missing, stale (> 7 days) or [force]d. */
    suspend fun ensure(force: Boolean = false): Boolean = withContext(Dispatchers.IO) {
        val season = currentSeason()
        val c = cached()
        val stale = c == null || c.first != season || System.currentTimeMillis() - c.second.lastModified() > 7L * 86_400_000
        if (force || stale) {
            // The new season's file may not be published yet: fall back to the previous one
            download(season) || (c == null && download(previousSeason(season)))
        }
        load()
    }

    private fun download(season: String): Boolean = runCatching {
        dir.mkdirs()
        val conn = URL("http://www.eibispace.de/dx/sked-$season.csv").openConnection() as HttpURLConnection
        conn.connectTimeout = 10_000
        conn.readTimeout = 30_000
        try {
            if (conn.responseCode != 200) return false
            val tmp = File(dir, "sked-$season.tmp")
            conn.inputStream.use { input -> tmp.outputStream().use { input.copyTo(it) } }
            tmp.renameTo(file(season))
        } finally {
            conn.disconnect()
        }
    }.getOrDefault(false)

    private fun load(): Boolean {
        val (season, f) = cached() ?: return false
        if (season == loadedSeason && entries.isNotEmpty()) return true
        val parsed = f.readLines(Charsets.ISO_8859_1).drop(1).mapNotNull(::parseLine).sortedBy { it.khz }
        entries = parsed
        loadedSeason = season
        return parsed.isNotEmpty()
    }

    /** Broadcasts on [freqHz] (± [toleranceHz]) at [now]. */
    fun lookup(freqHz: Double, toleranceHz: Double, now: ZonedDateTime): List<EiBiEntry> {
        val list = entries
        if (list.isEmpty()) return emptyList()
        val khz = freqHz / 1000.0
        val tol = toleranceHz / 1000.0
        // Binary search the first entry >= khz - tol
        var lo = 0
        var hi = list.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (list[mid].khz < khz - tol) lo = mid + 1 else hi = mid
        }
        val out = ArrayList<EiBiEntry>()
        var i = lo
        while (i < list.size && list[i].khz <= khz + tol) {
            if (list[i].activeAt(now)) out += list[i]
            i++
        }
        return out.sortedBy { abs(it.khz - khz) }
    }

    companion object {
        private val DAY_NAMES = listOf("Mo", "Tu", "We", "Th", "Fr", "Sa", "Su")

        fun parseLine(line: String): EiBiEntry? {
            val c = line.trimEnd('\r').split(';')
            if (c.size < 7) return null
            val khz = c[0].trim().toDoubleOrNull() ?: return null
            val time = c[1].trim()
            if (time.length < 9 || time[4] != '-') return null
            val start = hhmm(time.substring(0, 4)) ?: return null
            val end = hhmm(time.substring(5, 9)) ?: return null
            return EiBiEntry(khz, start, end, parseDays(c[2].trim()), c[3].trim(), c[4].trim(), c[5].trim(), c[6].trim())
        }

        private fun hhmm(s: String): Int? {
            val h = s.substring(0, 2).toIntOrNull() ?: return null
            val m = s.substring(2, 4).toIntOrNull() ?: return null
            return (h * 60 + m).coerceAtMost(24 * 60)
        }

        /** "" / "irr" → daily; "Mo-Fr", "We-Mo" (wraps), "SaSu", "Tu,Fr", "1245" (1 = Monday). */
        fun parseDays(s: String): Int {
            if (s.isEmpty()) return 0x7F
            if (s.all { it.isDigit() }) {
                return s.fold(0) { acc, ch -> val d = ch - '1'; if (d in 0..6) acc or (1 shl d) else acc }.takeIf { it != 0 } ?: 0x7F
            }
            var mask = 0
            for (part in s.split(',')) {
                val p = part.trim()
                val range = Regex("^([A-Z][a-z])-([A-Z][a-z])$").find(p)
                if (range != null) {
                    val a = DAY_NAMES.indexOf(range.groupValues[1])
                    val b = DAY_NAMES.indexOf(range.groupValues[2])
                    if (a < 0 || b < 0) continue
                    var d = a
                    while (true) {
                        mask = mask or (1 shl d)
                        if (d == b) break
                        d = (d + 1) % 7
                    }
                } else {
                    p.chunked(2).forEach { n -> DAY_NAMES.indexOf(n).takeIf { it >= 0 }?.let { mask = mask or (1 shl it) } }
                }
            }
            return if (mask == 0) 0x7F else mask // irregular / unknown formats: show it
        }
    }
}
