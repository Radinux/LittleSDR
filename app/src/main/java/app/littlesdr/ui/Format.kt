package app.littlesdr.ui

import java.util.Locale
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.log10
import kotlin.math.pow
import kotlin.math.roundToLong

/** "145.5125", "100.000", "7.074" (MHz, at least 3 decimals, trailing zeros trimmed). */
fun formatFrequency(hz: Double): String {
    var s = String.format(Locale.ROOT, "%.6f", hz / 1e6)
    while (s.endsWith("0") && s.length - s.indexOf('.') > 4) s = s.dropLast(1)
    return s
}

/** "12.5 kHz", "200 kHz", "2.4 MHz", "500 Hz" */
fun formatHz(hz: Double): String {
    val a = abs(hz)
    fun trim(v: Double) = String.format(Locale.ROOT, "%.3f", v).trimEnd('0').trimEnd('.')
    return when {
        a >= 1e9 -> "${trim(hz / 1e9)} GHz"
        a >= 1e6 -> "${trim(hz / 1e6)} MHz"
        a >= 1e3 -> "${trim(hz / 1e3)} kHz"
        else -> "${trim(hz)} Hz"
    }
}

/** Short axis label for a frequency, relative precision chosen from the tick step. */
fun axisLabel(hz: Double, step: Double): String {
    val decimals = when {
        step >= 1e6 -> 0
        step >= 1e5 -> 1
        step >= 1e4 -> 2
        step >= 1e3 -> 3
        else -> 4
    }
    return String.format(Locale.ROOT, "%.${decimals}f", hz / 1e6)
}

/** 1-2-5 sequence step giving about [targetTicks] ticks over [span]. */
fun niceStep(span: Double, targetTicks: Int): Double {
    if (span <= 0) return 1.0
    val raw = span / targetTicks
    val mag = 10.0.pow(floor(log10(raw)))
    val n = raw / mag
    val m = when {
        n < 1.5 -> 1.0
        n < 3.5 -> 2.0
        n < 7.5 -> 5.0
        else -> 10.0
    }
    return m * mag
}

fun snap(hz: Double, step: Double): Double = if (step <= 0) hz else (hz / step).roundToLong() * step
