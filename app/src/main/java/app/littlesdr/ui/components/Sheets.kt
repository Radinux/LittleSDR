package app.littlesdr.ui.components

import androidx.compose.foundation.clickable
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
import androidx.compose.material3.FilledTonalButton
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
import app.littlesdr.engine.Mode
import app.littlesdr.ui.formatFrequency
import app.littlesdr.ui.formatHz
import app.littlesdr.ui.theme.ReadoutStyle

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MemoriesSheet(
    memories: List<Memory>,
    currentFrequency: Double,
    onSave: (String) -> Unit,
    onRecall: (Memory) -> Unit,
    onRename: (Memory, String) -> Unit,
    onDelete: (Memory) -> Unit,
    onDismiss: () -> Unit,
) {
    var naming by remember { mutableStateOf<Memory?>(null) }
    var creating by remember { mutableStateOf(false) }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState()) {
        Column(Modifier.padding(horizontal = 20.dp).navigationBarsPadding()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Fréquences", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                FilledTonalButton(onClick = { creating = true }) {
                    Icon(Icons.Default.BookmarkAdd, null)
                    Spacer(Modifier.padding(4.dp))
                    Text("${formatFrequency(currentFrequency)} MHz")
                }
            }
            Spacer(Modifier.height(8.dp))
            if (memories.isEmpty()) {
                Column(
                    Modifier.fillMaxWidth().padding(vertical = 40.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Icon(Icons.Outlined.BookmarkBorder, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    Spacer(Modifier.height(8.dp))
                    Text("Aucune fréquence enregistrée", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            } else {
                LazyColumn(Modifier.heightIn(max = 520.dp)) {
                    items(memories, key = { it.id }) { m ->
                        var menu by remember { mutableStateOf(false) }
                        ListItem(
                            modifier = Modifier.clickable { onRecall(m); onDismiss() },
                            headlineContent = { Text(m.name) },
                            supportingContent = {
                                Text(
                                    "${formatFrequency(m.frequency)} MHz · ${Mode.of(m.mode)?.label ?: "?"} · ${formatHz(m.bandwidth.toDouble())}",
                                    style = ReadoutStyle,
                                )
                            },
                            leadingContent = { SuggestionChip(onClick = {}, label = { Text(Mode.of(m.mode)?.label ?: "?") }) },
                            trailingContent = {
                                Box {
                                    IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, "Options") }
                                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                                        DropdownMenuItem(
                                            text = { Text("Renommer") },
                                            leadingIcon = { Icon(Icons.Default.Edit, null) },
                                            onClick = { menu = false; naming = m },
                                        )
                                        DropdownMenuItem(
                                            text = { Text("Supprimer") },
                                            leadingIcon = { Icon(Icons.Default.Delete, null) },
                                            onClick = { menu = false; onDelete(m) },
                                        )
                                    }
                                }
                            },
                        )
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
        }
    }

    if (creating) {
        NameDialog(title = "Enregistrer la fréquence", initial = "", onDone = { onSave(it); creating = false }, onCancel = { creating = false })
    }
    naming?.let { m ->
        NameDialog(title = "Renommer", initial = m.name, onDone = { onRename(m, it); naming = null }, onCancel = { naming = null })
    }
}

@Composable
private fun NameDialog(title: String, initial: String, onDone: (String) -> Unit, onCancel: () -> Unit) {
    var text by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(title) },
        text = {
            OutlinedTextField(value = text, onValueChange = { text = it }, label = { Text("Nom") }, singleLine = true)
        },
        confirmButton = { TextButton(onClick = { onDone(text.trim()) }) { Text("OK") } },
        dismissButton = { TextButton(onClick = onCancel) { Text("Annuler") } },
    )
}
