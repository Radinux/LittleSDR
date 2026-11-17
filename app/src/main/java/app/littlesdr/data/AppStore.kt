package app.littlesdr.data

import android.content.Context
import androidx.datastore.core.CorruptionException
import androidx.datastore.core.DataStore
import androidx.datastore.core.DataStoreFactory
import androidx.datastore.core.Serializer
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.UUID

@Serializable
data class Memory(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val frequency: Double,
    val mode: Int,
    val bandwidth: Float,
    val createdAt: Long = System.currentTimeMillis(),
)

@Serializable
data class DisplaySettings(
    val dbMin: Float = -110f,
    val dbMax: Float = -30f,
    val spectrumFraction: Float = 0.35f,
    /** Waterfall palette, see ui/theme/Palettes.kt (0 = Turbo). */
    val palette: Int = 0,
)

@Serializable
data class Appearance(
    /** 0 = follow system, 1 = dark, 2 = light */
    val themeMode: Int = 1,
    /** Material You colors from the wallpaper (Android 12+) */
    val dynamicColor: Boolean = true,
    /** Index in ui/theme/Theme.kt Accent, used when dynamicColor is off */
    val accent: Int = 0,
)

@Serializable
data class RadioPrefs(
    /** NFM/WFM: follow the occupied bandwidth of the signal under the VFO */
    val autoBandwidth: Boolean = false,
)

@Serializable
data class OperatorPrefs(
    /** Keep the operator locator in sync with the phone's (approximate) location */
    val autoLocator: Boolean = false,
)

@Serializable
data class AppData(
    val memories: List<Memory> = emptyList(),
    val display: DisplaySettings = DisplaySettings(),
    val appearance: Appearance = Appearance(),
    val radio: RadioPrefs = RadioPrefs(),
    val operator: OperatorPrefs = OperatorPrefs(),
)

private object AppDataSerializer : Serializer<AppData> {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
    override val defaultValue = AppData()

    override suspend fun readFrom(input: InputStream): AppData = try {
        json.decodeFromString(AppData.serializer(), input.readBytes().decodeToString())
    } catch (e: SerializationException) {
        throw CorruptionException("Unreadable app data", e)
    }

    override suspend fun writeTo(t: AppData, output: OutputStream) {
        output.write(json.encodeToString(AppData.serializer(), t).encodeToByteArray())
    }
}

/** Memories (saved frequencies) and display preferences, persisted as JSON. */
class AppStore(context: Context) {
    private val store: DataStore<AppData> = DataStoreFactory.create(
        serializer = AppDataSerializer,
        produceFile = { File(context.filesDir, "littlesdr_data.json") },
    )

    val data: Flow<AppData> = store.data

    suspend fun addMemory(m: Memory) = store.updateData { d -> d.copy(memories = (d.memories + m).sortedBy { it.frequency }) }
    suspend fun updateMemory(m: Memory) = store.updateData { d -> d.copy(memories = d.memories.map { if (it.id == m.id) m else it }) }
    suspend fun deleteMemory(id: String) = store.updateData { d -> d.copy(memories = d.memories.filterNot { it.id == id }) }
    suspend fun restoreMemory(m: Memory) = addMemory(m)
    suspend fun setDisplay(s: DisplaySettings) = store.updateData { it.copy(display = s) }
    suspend fun setAppearance(a: Appearance) = store.updateData { it.copy(appearance = a) }
    suspend fun setRadio(r: RadioPrefs) = store.updateData { it.copy(radio = r) }
    suspend fun setOperator(o: OperatorPrefs) = store.updateData { it.copy(operator = o) }
}
