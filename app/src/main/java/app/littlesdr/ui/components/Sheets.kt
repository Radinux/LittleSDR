package app.littlesdr.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BookmarkAdd
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.BookmarkBorder
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import app.littlesdr.ui.theme.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SuggestionChip
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.littlesdr.data.Memory
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import app.littlesdr.engine.Mode
import app.littlesdr.ui.formatFrequency
import app.littlesdr.ui.formatHz
import app.littlesdr.ui.theme.ReadoutStyle

/** Memories tab: save the current frequency, tap a card to recall, menu to rename/delete. */
@Composable
fun MemoriesPanel(
    memories: List<Memory>,
    currentFrequency: Double,
    /** Name offered when saving (the station's RDS name), editable; empty when unknown */
    suggestedName: String = "",
    onSave: (String) -> Unit,
    onRecall: (Memory) -> Unit,
    onRename: (Memory, String) -> Unit,
    onDelete: (Memory) -> Unit,
) {
    var naming by remember { mutableStateOf<Memory?>(null) }
    var creating by remember { mutableStateOf(false) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        FilledTonalButton(onClick = { creating = true }, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Default.BookmarkAdd, null)
            Spacer(Modifier.padding(4.dp))
            Text(
                if (suggestedName.isNotBlank()) "Enregistrer $suggestedName · ${formatFrequency(currentFrequency)} MHz"
                else "Enregistrer ${formatFrequency(currentFrequency)} MHz",
                maxLines = 1,
            )
        }
        if (memories.isEmpty()) {
            Column(Modifier.fillMaxWidth().padding(vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(Icons.Outlined.BookmarkBorder, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(8.dp))
                Text("Aucune fréquence enregistrée", color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        memories.forEach { m ->
            var menu by remember(m.id) { mutableStateOf(false) }
            Row(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(18.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainer)
                    .clickable { onRecall(m) }
                    .padding(start = 14.dp, top = 8.dp, bottom = 8.dp, end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier
                        .clip(RoundedCornerShape(10.dp))
                        .background(MaterialTheme.colorScheme.secondaryContainer)
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                ) {
                    Text(Mode.of(m.mode)?.label ?: "?", style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSecondaryContainer)
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(m.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1)
                    Text("${formatFrequency(m.frequency)} MHz · ${formatHz(m.bandwidth.toDouble())}",
                        style = ReadoutStyle, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Box {
                    IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, "Options") }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(text = { Text("Renommer") }, leadingIcon = { Icon(Icons.Default.Edit, null) },
                            onClick = { menu = false; naming = m })
                        DropdownMenuItem(text = { Text("Supprimer") }, leadingIcon = { Icon(Icons.Default.Delete, null) },
                            onClick = { menu = false; onDelete(m) })
                    }
                }
            }
        }
    }

    if (creating) {
        NameDialog(title = "Enregistrer la fréquence", initial = suggestedName, onDone = { onSave(it); creating = false }, onCancel = { creating = false })
    }
    naming?.let { m ->
        NameDialog(title = "Renommer", initial = m.name, onDone = { onRename(m, it); naming = null }, onCancel = { naming = null })
    }
}

@Composable
private fun NameDialog(title: String, initial: String, onDone: (String) -> Unit, onCancel: () -> Unit) {
    // Pre-filled name selected: typing replaces it, a tap places the cursor to edit it
    var text by remember { mutableStateOf(TextFieldValue(initial, selection = TextRange(0, initial.length))) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = text, onValueChange = { text = it }, label = { Text("Nom") }, singleLine = true,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { onDone(text.text.trim()) }),
                modifier = Modifier.focusRequester(focus),
            )
        },
        confirmButton = { TextButton(onClick = { onDone(text.text.trim()) }) { Text("OK") } },
        dismissButton = { TextButton(onClick = onCancel) { Text("Annuler") } },
    )
}
