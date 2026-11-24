package app.littlesdr.engine

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.Locale

/** One aircraft heard on 1090 MHz (engine adsbJson) */
@Serializable
data class Aircraft(
    val icao: String = "",
    val callsign: String = "",
    /** Emitter category, "A3"...; empty when not sent */
    val category: String = "",
    /** ISO code of the country of registration, from the address block */
    val country: String = "",
    val squawk: String? = null,
    val emergency: Int = 0,
    /** Barometric altitude (ft) */
    val alt: Int? = null,
    val gnssAlt: Int? = null,
    val ground: Boolean? = null,
    /** kt: ground speed, or airspeed when [airspeed] */
    val speed: Double? = null,
    val airspeed: Boolean = false,
    val track: Double? = null,
    val heading: Double? = null,
    /** ft/min */
    val vrate: Int? = null,
    val lat: Double? = null,
    val lon: Double? = null,
    /** s since the last message / the last position */
    val seen: Double = 0.0,
    val seenPos: Double? = null,
    val msgs: Long = 0,
    val rssi: Double = 0.0,
    val nonIcao: Boolean = false,
    /** km and degrees from the receiver (operator locator) */
    val dist: Double? = null,
    val bearing: Double? = null,
    /** Recent positions: [lat, lon, alt ft or null] */
    val trail: List<List<Double?>> = emptyList(),
) {
    val name: String get() = callsign.ifBlank { icao }
    val position: GeoPoint? get() = if (lat != null && lon != null) GeoPoint(lat, lon) else null
    /** Direction the aircraft points to: track, else heading */
    val direction: Double? get() = track ?: heading
    val altitude: Int? get() = alt ?: gnssAlt
    val onGround: Boolean get() = ground == true
    /** Distance worth showing: not when the receiver is clearly elsewhere (network receiver, wrong locator) */
    val nearDist: Double? get() = dist?.takeIf { it < 800.0 }

    /** Distress: hijack, radio failure, emergency (squawk or the ADS-B emergency field) */
    val distress: String?
        get() = when {
            squawk == "7500" -> "Détournement (7500)"
            squawk == "7600" -> "Panne radio (7600)"
            squawk == "7700" -> "Urgence (7700)"
            emergency == 1 -> "Urgence"
            emergency == 2 -> "Urgence médicale"
            emergency == 3 -> "Carburant minimum"
            emergency == 4 -> "Panne radio"
            emergency == 5 -> "Intervention illicite"
            emergency == 6 -> "Carburant (déroutement)"
            else -> null
        }

    /** Kind of aircraft from the emitter category */
    val categoryLabel: String
        get() = when (category) {
            "A1" -> "Avion léger"
            "A2" -> "Petit avion"
            "A3" -> "Avion de ligne"
            "A4" -> "Gros porteur (757)"
            "A5" -> "Gros porteur"
            "A6" -> "Haute performance"
            "A7" -> "Hélicoptère"
            "B1" -> "Planeur"
            "B2" -> "Ballon, dirigeable"
            "B3" -> "Parachutiste"
            "B4" -> "ULM"
            "B6" -> "Drone"
            "B7" -> "Engin spatial"
            "C1" -> "Véhicule d'urgence"
            "C2" -> "Véhicule de service"
            "C3", "C4", "C5" -> "Obstacle"
            else -> ""
        }

    /** Country name (French) and flag */
    val countryName: String get() = if (country.isBlank()) "" else Locale("", country).getDisplayCountry(Locale.FRENCH)
    val flag: String
        get() = if (country.length != 2) "" else country.uppercase().map { Character.toChars(0x1F1E6 + (it - 'A')) }.joinToString("") { String(it) }
}

@Serializable
data class AdsbState(
    val active: Boolean = false,
    val messages: Long = 0,
    /** Messages per second */
    val rate: Double = 0.0,
    val repaired: Long = 0,
    /** Receiver position [lat, lon] (from the operator locator), when set */
    val home: List<Double>? = null,
    val aircraft: List<Aircraft> = emptyList(),
) {
    val homePoint: GeoPoint? get() = home?.takeIf { it.size == 2 }?.let { GeoPoint(it[0], it[1]) }

    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true }
        fun parse(s: String): AdsbState = runCatching { json.decodeFromString(serializer(), s) }.getOrDefault(AdsbState())
    }
}

/** ADS-B and Mode S: 1090 MHz */
const val ADSB_FREQUENCY = 1_090_000_000.0
