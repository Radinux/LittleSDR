package app.littlesdr.ui.theme

import androidx.compose.foundation.BorderStroke
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/** Thin outline that sets the app's panels ("pills") apart from the background */
@Composable
fun pillBorder(): BorderStroke = BorderStroke(1.dp, pillOutline())

/** The panels' outline color (tinted with the accent, like the surfaces) */
@Composable
fun pillOutline(): Color = MaterialTheme.colorScheme.outlineVariant
