package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** RDS of the current WFM station (engine::rdsJson, decoded by SDR++Brown's WFM demodulator). */
@Serializable
data class RdsInfo(
    val active: Boolean = false,
    /** Programme Service name, e.g. "FIP" */
    val ps: String = "",
    /** RadioText (title / artist / messages) */
    val rt: String = "",
    val pi: String = "",
    val callsign: String = "",
    /** Programme type, e.g. "Pop Music" */
    val pty: String = "",
) {
    val hasData: Boolean get() = ps.isNotBlank() || rt.isNotBlank()

    companion object {
        private val json = Json { ignoreUnknownKeys = true }
        fun parse(s: String): RdsInfo = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(RdsInfo())
    }
}
