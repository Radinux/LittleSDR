package app.littlesdr.ui.theme

import androidx.compose.ui.graphics.Color

/**
 * Kotlin mirror of colormap() in cpp/shaders/common.glsl, used for the previews in Settings.
 * Order must match the shader's palette index.
 */
enum class Palette(val label: String) {
    Turbo("Turbo"), Viridis("Viridis"), Inferno("Inferno"), Magma("Magma"), Classic("Classique"), Gray("Niveaux de gris"),
    Plasma("Plasma"), Ocean("Océan"), Fire("Feu"), Aurora("Aurore"), Phosphor("Phosphore"), Ice("Glace"), Neon("Néon"), Copper("Cuivre"), Rainbow("Arc-en-ciel");

    fun color(t: Float): Color {
        val x = t.coerceIn(0f, 1f).toDouble()
        val rgb = when (this) {
            Turbo -> turbo(x)
            Viridis -> poly6(x, VIRIDIS)
            Inferno -> poly6(x, INFERNO)
            Magma -> poly6(x, MAGMA)
            Classic -> classic(x)
            Gray -> doubleArrayOf(x, x, x)
            else -> stops6(STOPS[ordinal - Plasma.ordinal], x)
        }
        return Color(rgb[0].toFloat().coerceIn(0f, 1f), rgb[1].toFloat().coerceIn(0f, 1f), rgb[2].toFloat().coerceIn(0f, 1f))
    }

    /** Sampled stops for a gradient preview. */
    fun stops(n: Int = 16): List<Color> = List(n) { color(it / (n - 1f)) }

    companion object {
        fun of(i: Int) = entries.getOrElse(i) { Turbo }

        private fun turbo(x: Double): DoubleArray {
            val x2 = x * x; val x3 = x2 * x; val x4 = x2 * x2; val x5 = x4 * x
            return doubleArrayOf(
                0.13572138 + 4.61539260 * x - 42.66032258 * x2 + 132.13108234 * x3 - 152.94239396 * x4 + 59.28637943 * x5,
                0.09140261 + 2.19418839 * x + 4.84296658 * x2 - 14.18503333 * x3 + 4.27729857 * x4 + 2.82956604 * x5,
                0.10667330 + 12.64194608 * x - 60.58204836 * x2 + 110.36276771 * x3 - 89.90310912 * x4 + 27.34824973 * x5,
            )
        }

        private fun poly6(t: Double, c: Array<DoubleArray>): DoubleArray = DoubleArray(3) { k ->
            var v = c[6][k]
            for (i in 5 downTo 0) v = c[i][k] + t * v
            v
        }

        private fun classic(t: Double): DoubleArray {
            val k = arrayOf(
                doubleArrayOf(0.0, 0.0, 0.12), doubleArrayOf(0.0, 0.25, 1.0), doubleArrayOf(0.0, 1.0, 1.0),
                doubleArrayOf(1.0, 1.0, 0.0), doubleArrayOf(1.0, 0.0, 0.0),
            )
            val x = t * 4.0
            val i = minOf(x.toInt(), 3)
            val f = x - i
            return DoubleArray(3) { k[i][it] + (k[i + 1][it] - k[i][it]) * f }
        }

        /** 6 evenly spaced stops per palette, same as STOPS in common.glsl */
        private fun stops6(k: Array<DoubleArray>, t: Double): DoubleArray {
            val x = t * 5.0
            val i = minOf(x.toInt(), 4)
            val f = x - i
            return DoubleArray(3) { k[i][it] + (k[i + 1][it] - k[i][it]) * f }
        }

        private val STOPS = arrayOf(
            arrayOf(doubleArrayOf(0.050, 0.030, 0.528), doubleArrayOf(0.417, 0.000, 0.658), doubleArrayOf(0.693, 0.165, 0.565), doubleArrayOf(0.881, 0.393, 0.383), doubleArrayOf(0.988, 0.652, 0.211), doubleArrayOf(0.940, 0.975, 0.131)), // Plasma,
            arrayOf(doubleArrayOf(0.000, 0.020, 0.080), doubleArrayOf(0.000, 0.100, 0.300), doubleArrayOf(0.000, 0.330, 0.550), doubleArrayOf(0.050, 0.620, 0.700), doubleArrayOf(0.450, 0.880, 0.850), doubleArrayOf(0.950, 1.000, 1.000)), // Océan,
            arrayOf(doubleArrayOf(0.000, 0.000, 0.000), doubleArrayOf(0.300, 0.000, 0.000), doubleArrayOf(0.700, 0.100, 0.000), doubleArrayOf(0.950, 0.400, 0.000), doubleArrayOf(1.000, 0.780, 0.200), doubleArrayOf(1.000, 1.000, 0.850)), // Feu,
            arrayOf(doubleArrayOf(0.020, 0.000, 0.060), doubleArrayOf(0.200, 0.020, 0.350), doubleArrayOf(0.100, 0.350, 0.550), doubleArrayOf(0.050, 0.700, 0.450), doubleArrayOf(0.550, 0.950, 0.400), doubleArrayOf(0.950, 1.000, 0.850)), // Aurore,
            arrayOf(doubleArrayOf(0.000, 0.020, 0.000), doubleArrayOf(0.000, 0.150, 0.030), doubleArrayOf(0.020, 0.350, 0.080), doubleArrayOf(0.100, 0.620, 0.150), doubleArrayOf(0.400, 0.900, 0.350), doubleArrayOf(0.850, 1.000, 0.800)), // Phosphore,
            arrayOf(doubleArrayOf(0.000, 0.000, 0.030), doubleArrayOf(0.020, 0.060, 0.200), doubleArrayOf(0.080, 0.220, 0.480), doubleArrayOf(0.250, 0.500, 0.800), doubleArrayOf(0.600, 0.820, 0.970), doubleArrayOf(1.000, 1.000, 1.000)), // Glace,
            arrayOf(doubleArrayOf(0.020, 0.000, 0.050), doubleArrayOf(0.250, 0.000, 0.400), doubleArrayOf(0.700, 0.050, 0.650), doubleArrayOf(0.950, 0.300, 0.600), doubleArrayOf(0.300, 0.850, 0.950), doubleArrayOf(0.900, 1.000, 1.000)), // Néon,
            arrayOf(doubleArrayOf(0.020, 0.010, 0.000), doubleArrayOf(0.200, 0.080, 0.020), doubleArrayOf(0.450, 0.200, 0.060), doubleArrayOf(0.720, 0.400, 0.150), doubleArrayOf(0.920, 0.660, 0.350), doubleArrayOf(1.000, 0.920, 0.750)), // Cuivre,
            arrayOf(doubleArrayOf(0.000, 0.000, 0.250), doubleArrayOf(0.000, 0.000, 1.000), doubleArrayOf(0.000, 1.000, 1.000), doubleArrayOf(0.000, 1.000, 0.000), doubleArrayOf(1.000, 1.000, 0.000), doubleArrayOf(1.000, 0.000, 0.000)), // Arc-en-ciel,
        )

        private val VIRIDIS = arrayOf(
            doubleArrayOf(0.2777273272234177, 0.005407344544966578, 0.3340998053353061),
            doubleArrayOf(0.1050930431085774, 1.404613529898575, 1.384590162594685),
            doubleArrayOf(-0.3308618287255563, 0.214847559468213, 0.09509516302823659),
            doubleArrayOf(-4.634230498983486, -5.799100973351585, -19.33244095627987),
            doubleArrayOf(6.228269936347081, 14.17993336680509, 56.69055260068105),
            doubleArrayOf(4.776384997670288, -13.74514537774601, -65.35303263337234),
            doubleArrayOf(-5.435455855934631, 4.645852612178535, 26.3124352495832),
        )
        private val INFERNO = arrayOf(
            doubleArrayOf(0.0002189403691192265, 0.001651004631001012, -0.01948089843709184),
            doubleArrayOf(0.1065134194856116, 0.5639564367884091, 3.932712388889277),
            doubleArrayOf(11.60249308247187, -3.972853965665698, -15.9423941062914),
            doubleArrayOf(-41.70399613139459, 17.43639888205313, 44.35414519872813),
            doubleArrayOf(77.162935699427, -33.40235894210092, -81.80730925738993),
            doubleArrayOf(-71.31942824499214, 32.62606426397723, 73.20951985803202),
            doubleArrayOf(25.13112622477341, -12.24266895238567, -23.07032500287172),
        )
        private val MAGMA = arrayOf(
            doubleArrayOf(-0.002136485053939582, -0.000749655052795221, -0.005386127855323933),
            doubleArrayOf(0.2516605407371642, 0.6775232436837668, 2.494026599312351),
            doubleArrayOf(8.353717279216625, -3.577719514958484, 0.3144679030132573),
            doubleArrayOf(-27.66873308576866, 14.26473078096533, -13.64921318813922),
            doubleArrayOf(52.17613981234068, -27.94360607168351, 12.94416944238394),
            doubleArrayOf(-50.76852536473588, 29.04658282127291, 4.23415299384598),
            doubleArrayOf(18.65570506591883, -11.48977351997711, -5.601961508734096),
        )
    }
}
