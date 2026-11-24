package app.littlesdr.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material.icons.filled.Link
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.littlesdr.data.KiwiServer
import app.littlesdr.engine.GeoPoint
import kotlin.math.roundToInt

/** Button shown under the KiwiSDR source: opens the receiver picker. */
@Composable
fun KiwiServerButton(
    servers: List<KiwiServer>,
    loading: Boolean,
    position: GeoPoint?,
    refused: Set<String> = emptySet(),
    onLoad: (force: Boolean) -> Unit,
    onPick: (hostPort: String, name: String) -> Unit,
) {
    var open by rememberSaveable { mutableStateOf(false) }
    FilledTonalButton(onClick = { open = true }, modifier = Modifier.fillMaxWidth()) {
        Icon(Icons.Default.Public, null, Modifier.size(18.dp))
        Text("Choisir un serveur KiwiSDR", Modifier.padding(start = 8.dp))
    }
    if (open) {
        KiwiPickerDialog(
            servers = servers, loading = loading, position = position, refused = refused, onLoad = onLoad,
            onPick = { hp, name -> onPick(hp, name); open = false },
            onDismiss = { open = false },
        )
    }
}

/**
 * Large centered pop-up (made for landscape): search, filter and manual address on top, the
 * receiver list on the whole remaining height, one line per receiver.
 */
@Composable
private fun KiwiPickerDialog(
    servers: List<KiwiServer>,
    loading: Boolean,
    position: GeoPoint?,
    refused: Set<String>,
    onLoad: (force: Boolean) -> Unit,
    onPick: (hostPort: String, name: String) -> Unit,
    onDismiss: () -> Unit,
) {
    LaunchedEffect(Unit) { onLoad(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var freeOnly by rememberSaveable { mutableStateOf(true) }
    // Receivers behind the kiwisdr.com proxy are often unreachable: hidden unless asked for
    var directOnly by rememberSaveable { mutableStateOf(true) }
    var manual by rememberSaveable { mutableStateOf("") }
    var manualOpen by rememberSaveable { mutableStateOf(false) }
    val listState = rememberLazyListState()

    // Nearest first when the operator locator is known, otherwise by name
    val shown = remember(servers, query, freeOnly, directOnly, position, refused) {
        val q = query.trim().lowercase()
        servers.asSequence()
            .filter { !freeOnly || it.free }
            .filter { !directOnly || !it.proxied }
            .filter { q.isEmpty() || q in it.name.lowercase() || q in it.location.lowercase() || q in it.hostPort.lowercase() }
            .map { it to position?.let { p -> it.distanceKm(p.lat, p.lon) } }
            // Servers that already turned the app away go last
            .sortedWith(compareBy<Pair<KiwiServer, Double?>> { it.first.hostPort in refused }.thenBy { it.second ?: Double.MAX_VALUE }.thenBy { sortName(it.first) })
            .toList()
    }
    LaunchedEffect(query, freeOnly, directOnly) { listState.scrollToItem(0) }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(
            shape = RoundedCornerShape(24.dp),
            color = MaterialTheme.colorScheme.surfaceContainer,
            modifier = Modifier.fillMaxWidth(0.94f).fillMaxHeight(0.94f),
        ) {
            Column(Modifier.fillMaxSize().padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 8.dp)) {
                // Title, count, refresh, close
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Serveurs KiwiSDR", style = MaterialTheme.typography.titleLarge)
                    Text(
                        when {
                            loading && servers.isEmpty() -> "  ·  chargement de la liste publique…"
                            position == null -> "  ·  ${shown.size}/${servers.size} · par nom (locator absent)"
                            else -> "  ·  ${shown.size}/${servers.size} · les plus proches d'abord"
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(onClick = { manualOpen = !manualOpen }) {
                        Icon(Icons.Default.Link, "Adresse manuelle", tint = if (manualOpen) MaterialTheme.colorScheme.primary else LocalContentColor.current)
                    }
                    if (loading) CircularProgressIndicator(Modifier.padding(12.dp).size(24.dp), strokeWidth = 2.dp)
                    else IconButton(onClick = { onLoad(true) }) { Icon(Icons.Default.Refresh, "Actualiser la liste") }
                    IconButton(onClick = onDismiss) { Icon(Icons.Default.Close, "Fermer") }
                }

                // Search + filters on one line (placeholders on one line too: with a large
                // system font they used to wrap and eat the list's height)
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    modifier = Modifier.padding(end = 8.dp, bottom = 6.dp),
                ) {
                    OutlinedTextField(
                        value = query, onValueChange = { query = it },
                        placeholder = { Text("Nom, ville, pays…", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        leadingIcon = { Icon(Icons.Default.Search, null) },
                        trailingIcon = if (query.isNotEmpty()) {
                            { IconButton(onClick = { query = "" }) { Icon(Icons.Default.Close, "Effacer") } }
                        } else null,
                        singleLine = true,
                        shape = RoundedCornerShape(16.dp),
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                        modifier = Modifier.weight(1f),
                    )
                    FilterChip(selected = freeOnly, onClick = { freeOnly = !freeOnly }, label = { Text("Places libres", maxLines = 1) })
                    FilterChip(selected = directOnly, onClick = { directOnly = !directOnly }, label = { Text("Accès direct", maxLines = 1) })
                }
                // Manual address: only when asked for (link button in the title)
                if (manualOpen) Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    modifier = Modifier.padding(end = 8.dp, bottom = 6.dp),
                ) {
                    OutlinedTextField(
                        value = manual, onValueChange = { manual = it },
                        placeholder = { Text("hôte:8073", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        leadingIcon = { Icon(Icons.Default.Link, null) },
                        singleLine = true,
                        shape = RoundedCornerShape(16.dp),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(onDone = { manualHostPort(manual)?.let { onPick(it, it) } }),
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(onClick = { manualHostPort(manual)?.let { onPick(it, it) } }, enabled = manualHostPort(manual) != null) {
                        Text("Utiliser")
                    }
                }
                HorizontalDivider()

                LazyColumn(state = listState, modifier = Modifier.fillMaxWidth().weight(1f)) {
                    items(shown, key = { it.first.hostPort }) { (s, km) ->
                        KiwiRow(s, km, s.hostPort in refused) { onPick(s.hostPort, s.name) }
                    }
                    if (shown.isEmpty() && !loading) {
                        item {
                            Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
                                Text(
                                    if (servers.isEmpty()) "Liste indisponible (connexion internet ?)" else "Aucun récepteur",
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

/** One line: name | place | distance | listeners. */
@Composable
private fun KiwiRow(s: KiwiServer, km: Double?, refused: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 10.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            s.name.ifBlank { s.hostPort }, style = MaterialTheme.typography.bodyLarge,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1.4f),
        )
        Text(
            when {
                refused -> "a refusé l'app (places prises)"
                s.proxied -> "${s.location} · proxy"
                else -> s.location
            },
            style = MaterialTheme.typography.bodyMedium,
            color = if (refused) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
        )
        Text(
            when {
                km == null -> ""
                km < 10 -> "%.1f km".format(km)
                else -> "${km.roundToInt()} km"
            },
            style = MaterialTheme.typography.bodyMedium, textAlign = TextAlign.End, modifier = Modifier.width(76.dp),
        )
        Text(
            "${s.users}/${s.usersMax}",
            style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium, textAlign = TextAlign.End,
            color = if (s.free) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error,
            modifier = Modifier.width(44.dp),
        )
    }
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f))
}

/** Name for the alphabetical order: leading symbols and emoji ("##", "- ", "🧿") skipped */
private fun sortName(s: KiwiServer): String = s.name.dropWhile { !it.isLetterOrDigit() }.lowercase().ifEmpty { s.hostPort }

/** "host", "host:port" or "http://host:port/" -> "host:port" (KiwiSDR default port 8073). */
private fun manualHostPort(text: String): String? {
    val t = text.trim().removePrefix("http://").removePrefix("ws://").substringBefore('/')
    if (t.isBlank() || ' ' in t) return null
    return if (':' in t) t.takeIf { it.substringAfter(':').toIntOrNull() != null } else "$t:8073"
}
