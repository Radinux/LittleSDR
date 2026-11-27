package app.littlesdr.ui.theme

import android.graphics.BlurMaskFilter
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.RowScope
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonElevation
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.IconButtonColors
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LocalMinimumInteractiveComponentSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SelectableChipColors
import androidx.compose.material3.SelectableChipElevation
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.addOutline
import androidx.compose.ui.graphics.asAndroidPath
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/*
 * Neumorphism: the buttons look moulded in the panel. Outside, a soft shadow down-right and a
 * faint light up-left lift them; inside, past a flat rim, the same light the other way round
 * hollows them out (shadow under the top-left edge of the dip, light along the bottom-right one).
 *
 * The Material buttons below are drop-in replacements (same parameters): only the import
 * changes. The touch target keeps its 48 dp (around the button, as Material does), but the
 * effect follows the visible shape.
 */

/**
 * Neumorphic relief on a component of this [shape]: raised outside ([raised]), hollow inside.
 * The hollow starts [inset] inside the edge: a flat rim, then the dip. Drawn around and over
 * the component (the inner shadow lies on its face, under nothing).
 */
@Composable
fun Modifier.neu(shape: Shape, raised: Boolean = true, depth: Dp = 2.5.dp, inset: Dp = 3.5.dp): Modifier {
    val dark = MaterialTheme.colorScheme.surface.luminance() < 0.5f
    val shadow = Color.Black.copy(alpha = if (dark) 0.55f else 0.20f).toArgb()
    val light = Color.White.copy(alpha = if (dark) 0.10f else 0.90f).toArgb()
    val innerShadow = Color.Black.copy(alpha = if (dark) 0.60f else 0.22f).toArgb()
    val innerLight = Color.White.copy(alpha = if (dark) 0.12f else 0.75f).toArgb()
    return drawWithCache {
        val d = depth.toPx()
        val path = Path().apply { addOutline(shape.createOutline(size, layoutDirection, this@drawWithCache)) }
        // The hollow: the same shape, a rim's width smaller all round (the whole face if too small)
        val i = inset.toPx().coerceAtMost(minOf(size.width, size.height) / 4f)
        val hollow = Path().apply {
            addOutline(shape.createOutline(Size(size.width - 2 * i, size.height - 2 * i), layoutDirection, this@drawWithCache))
            translate(Offset(i, i))
        }
        // Everything outside the hollow: shifted, its blurred edge falls inside (inner shadow)
        val frame = Path().apply {
            fillType = PathFillType.EvenOdd
            addRect(Rect(-size.width, -size.height, size.width * 2, size.height * 2))
            addPath(hollow)
        }
        val p = path.asAndroidPath()
        val h = hollow.asAndroidPath()
        val f = frame.asAndroidPath()
        fun paint(argb: Int, blur: Float) = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
            color = argb
            maskFilter = BlurMaskFilter(blur, BlurMaskFilter.Blur.NORMAL)
        }
        val outShadow = paint(shadow, d * 2f)
        val outLight = paint(light, d * 2f)
        val inShadow = paint(innerShadow, d * 1.6f)
        val inLight = paint(innerLight, d * 1.6f)
        onDrawWithContent {
            drawIntoCanvas { c ->
                val nc = c.nativeCanvas
                if (raised) {
                    nc.save(); nc.translate(d, d); nc.drawPath(p, outShadow); nc.restore()
                    nc.save(); nc.translate(-d, -d); nc.drawPath(p, outLight); nc.restore()
                }
            }
            drawContent()
            drawIntoCanvas { c ->
                val nc = c.nativeCanvas
                nc.save()
                nc.clipPath(h)
                nc.save(); nc.translate(d, d); nc.drawPath(f, inShadow); nc.restore()
                nc.save(); nc.translate(-d, -d); nc.drawPath(f, inLight); nc.restore()
                nc.restore()
            }
        }
    }
}

/**
 * Hosts a Material component with the effect: [modifier] (size, weight…) applies to the host,
 * which passes its minimum size on; the component draws at its visible size (its own 48 dp
 * margin is off, the host keeps the room instead).
 */
@Composable
private fun NeuHost(modifier: Modifier, shape: Shape, raised: Boolean, content: @Composable (Modifier) -> Unit) {
    Box(modifier.minimumInteractiveComponentSize(), contentAlignment = Alignment.Center, propagateMinConstraints = true) {
        CompositionLocalProvider(LocalMinimumInteractiveComponentSize provides Dp.Unspecified) {
            content(Modifier.neu(shape, raised))
        }
    }
}

@Composable
fun FilledIconButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = IconButtonDefaults.filledShape,
    colors: IconButtonColors = IconButtonDefaults.filledIconButtonColors(),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable () -> Unit,
) = NeuHost(modifier, shape, raised = true) { m ->
    androidx.compose.material3.FilledIconButton(onClick, m, enabled, shape, colors, interactionSource, content)
}

@Composable
fun FilledTonalIconButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = IconButtonDefaults.filledShape,
    colors: IconButtonColors = IconButtonDefaults.filledTonalIconButtonColors(),
    interactionSource: MutableInteractionSource? = null,
    content: @Composable () -> Unit,
) = NeuHost(modifier, shape, raised = true) { m ->
    androidx.compose.material3.FilledTonalIconButton(onClick, m, enabled, shape, colors, interactionSource, content)
}

@Composable
fun Button(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = ButtonDefaults.shape,
    colors: ButtonColors = ButtonDefaults.buttonColors(),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = null,
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = NeuHost(modifier, shape, raised = true) { m ->
    androidx.compose.material3.Button(onClick, m, enabled, shape, colors, elevation, border, contentPadding, interactionSource, content)
}

@Composable
fun FilledTonalButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = ButtonDefaults.filledTonalShape,
    colors: ButtonColors = ButtonDefaults.filledTonalButtonColors(),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = null,
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = NeuHost(modifier, shape, raised = true) { m ->
    androidx.compose.material3.FilledTonalButton(onClick, m, enabled, shape, colors, elevation, border, contentPadding, interactionSource, content)
}

/** Outlined: no relief outside (it has no fill to lift), hollow inside */
@Composable
fun OutlinedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = ButtonDefaults.outlinedShape,
    colors: ButtonColors = ButtonDefaults.outlinedButtonColors(),
    elevation: ButtonElevation? = null,
    border: BorderStroke? = ButtonDefaults.outlinedButtonBorder(enabled),
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    interactionSource: MutableInteractionSource? = null,
    content: @Composable RowScope.() -> Unit,
) = NeuHost(modifier, shape, raised = false) { m ->
    androidx.compose.material3.OutlinedButton(onClick, m, enabled, shape, colors, elevation, border, contentPadding, interactionSource, content)
}

/** Chips: hollow inside; the selected one also stands out */
@Composable
fun FilterChip(
    selected: Boolean,
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leadingIcon: @Composable (() -> Unit)? = null,
    trailingIcon: @Composable (() -> Unit)? = null,
    shape: Shape = FilterChipDefaults.shape,
    colors: SelectableChipColors = FilterChipDefaults.filterChipColors(),
    elevation: SelectableChipElevation? = null,
    border: BorderStroke? = FilterChipDefaults.filterChipBorder(enabled, selected),
    interactionSource: MutableInteractionSource? = null,
) = NeuHost(modifier, shape, raised = selected) { m ->
    androidx.compose.material3.FilterChip(selected, onClick, label, m, enabled, leadingIcon, trailingIcon, shape, colors, elevation, border, interactionSource)
}
