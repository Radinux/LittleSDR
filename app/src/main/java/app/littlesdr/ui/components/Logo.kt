package app.littlesdr.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.sp

/**
 * A station logo filling its (clipped) box: a roughly square logo fills it whole; a wide or
 * tall one (a name in letters) would lose its ends, so it fits the box on its own background
 * colour, taken from its edge: no grey margin around a little square.
 */
@Composable
fun LogoFill(logo: ImageBitmap, modifier: Modifier = Modifier, description: String? = null) {
    val ratio = logo.width.toFloat() / logo.height.coerceAtLeast(1)
    if (ratio in 0.8f..1.25f) {
        Image(logo, description, contentScale = ContentScale.Crop, modifier = modifier.fillMaxSize())
    } else {
        val edge = remember(logo) { edgeColor(logo) }
        Box(modifier.fillMaxSize().background(edge), contentAlignment = Alignment.Center) {
            Image(logo, description, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize())
        }
    }
}

/** The colour around a logo: its corners, ignoring transparent ones (then black, as the cards) */
private fun edgeColor(img: ImageBitmap): Color {
    val px = img.toPixelMap()
    val (w, h) = img.width to img.height
    val corners = listOf(px[1, 1], px[w - 2, 1], px[1, h - 2], px[w - 2, h - 2]).filter { it.alpha > 0.5f }
    if (corners.isEmpty()) return Color.Transparent
    return Color(corners.map { it.red }.average().toFloat(), corners.map { it.green }.average().toFloat(), corners.map { it.blue }.average().toFloat())
}

/**
 * Text at the largest size (from [style]'s, down to [min]) where no word is cut in two and
 * the whole fits in [maxLines]: "NOSTALGIE" shrinks a little in a narrow tile instead of
 * breaking into "NOSTALGI / E" (large system text sizes, narrow phones).
 */
@Composable
fun WordFitText(
    text: String,
    modifier: Modifier = Modifier,
    style: TextStyle = LocalTextStyle.current,
    color: Color = Color.Unspecified,
    maxLines: Int = 2,
    min: TextUnit = 10.sp,
) {
    val measurer = rememberTextMeasurer()
    BoxWithConstraints(modifier) {
        val width = constraints.maxWidth
        val size = remember(text, style, width) {
            var s = style.fontSize.value
            val words = text.split(' ').filter { it.isNotBlank() }
            while (s > min.value) {
                val st = style.copy(fontSize = s.sp)
                val widest = words.maxOfOrNull { measurer.measure(it, st, maxLines = 1, softWrap = false).size.width } ?: 0
                val lines = measurer.measure(text, st, constraints = Constraints(maxWidth = width)).lineCount
                if (widest <= width && lines <= maxLines) break
                s -= 1f
            }
            s.sp
        }
        Text(text, style = style.copy(fontSize = size), color = color, maxLines = maxLines, overflow = TextOverflow.Ellipsis)
    }
}
