package app.littlesdr.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.littlesdr.data.KiwiServer
import app.littlesdr.engine.GeoPoint
import kotlin.math.roundToInt

/** Button shown under the KiwiSDR source: opens the receiver picker. */
@Composable
fun KiwiServerButton(
    servers: List<KiwiServer>,
    loading: Boolean,
    position: GeoPoint?,
    onLoad: (force: Boolean) -> Unit,
    onPick: (hostPort: String, name: String) -> Unit,
) {
    var open by rememberSaveable { mutableStateOf(false) }
    FilledTonalButton(onClick = { open = true }, modifier = Modifier.fillMaxWidth()) {
        Icon(Icons.Default.Public, null, Modifier.size(18.dp))
        Text("Choisir un serveur KiwiSDR", Modifier.padding(start = 8.dp))
    }
    if (open) {
        KiwiPickerSheet(
            servers = servers, loading = loading, position = position, onLoad = onLoad,
            onPick = { hp, name -> onPick(hp, name); open = false },
            onDismiss = { open = false },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun KiwiPickerSheet(
    servers: List<KiwiServer>,
    loading: Boolean,
    position: GeoPoint?,
    onLoad: (force: Boolean) -> Unit,
    onPick: (hostPort: String, name: String) -> Unit,
    onDismiss: () -> Unit,
) {
    LaunchedEffect(Unit) { onLoad(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var freeOnly by rememberSaveable { mutableStateOf(true) }
    var manual by rememberSaveable { mutableStateOf("") }

    // Nearest first when the operator locator is known, otherwise by name
    val shown = remember(servers, query, freeOnly, position) {
        val q = query.trim().lowercase()
        servers.asSequence()
            .filter { !freeOnly || it.free }
            .filter { q.isEmpty() || q in it.name.lowercase() || q in it.location.lowercase() || q in it.hostPort.lowercase() }
            .map { it to position?.let { p -> it.distanceKm(p.lat, p.lon) } }
            .sortedWith(compareBy<Pair<KiwiServer, Double?>> { it.second ?: Double.MAX_VALUE }.thenBy { it.first.name })
            .take(300)
            .toList()
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 20.dp).navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Serveurs KiwiSDR", style = MaterialTheme.typography.titleLarge)
                    Text(
                        when {
                            loading && servers.isEmpty() -> "Chargement de la liste publique…"
                            position == null -> "${servers.size} récepteurs · indique ton locator dans les paramètres pour trier par distance"
                            else -> "${servers.size} récepteurs · les plus proches d'abord"
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (loading) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                else IconButton(onClick = { onLoad(true) }) { Icon(Icons.Default.Refresh, "Actualiser la liste") }
            }

            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = query, onValueChange = { query = it },
                    placeholder = { Text("Nom, ville, pays…") },
                    leadingIcon = { Icon(Icons.Default.Search, null) },
                    singleLine = true,
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.weight(1f),
                )
                FilterChip(selected = freeOnly, onClick = { freeOnly = !freeOnly }, label = { Text("Places libres") })
            }

            // Address typed by hand (private or unlisted receivers)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = manual, onValueChange = { manual = it },
                    placeholder = { Text("Ou une adresse : hôte:8073") },
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

            LazyColumn(Modifier.fillMaxWidth().height(420.dp)) {
                items(shown, key = { it.first.hostPort + it.first.name }) { (s, km) ->
                    KiwiRow(s, km) { onPick(s.hostPort, s.name) }
                }
                if (shown.isEmpty() && !loading) {
                    item {
                        Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
                            Text("Aucun récepteur", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun KiwiRow(s: KiwiServer, km: Double?, onClick: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 10.dp, horizontal = 4.dp),
    ) {
        Text(s.name.ifBlank { s.hostPort }, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
        val details = buildList {
            if (s.location.isNotBlank()) add(s.location)
            if (km != null) add(if (km < 10) "%.1f km".format(km) else "${km.roundToInt()} km")
            add("${s.users}/${s.usersMax} auditeurs")
        }.joinToString(" · ")
        Text(
            details,
            style = MaterialTheme.typography.bodySmall,
            color = if (s.free) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
    }
}

/** "host", "host:port" or "http://host:port/" -> "host:port" (KiwiSDR default port 8073). */
private fun manualHostPort(text: String): String? {
    val t = text.trim().removePrefix("http://").removePrefix("ws://").substringBefore('/')
    if (t.isBlank() || ' ' in t) return null
    return if (':' in t) t.takeIf { it.substringAfter(':').toIntOrNull() != null } else "$t:8073"
}
