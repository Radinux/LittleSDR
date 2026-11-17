package app.littlesdr.engine

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.float
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Widgets of a source's settings menu, as recorded by SDR++'s SmGui (server-mode GUI)
 * and converted to JSON by smgui_json.cpp. Rendered with Material 3 by SmGuiPanel.
 */
sealed interface SmWidget {
    val disabled: Boolean
    val sameLine: Boolean
    val title: String?

    data class Combo(val id: String, val label: String, val value: Int, val items: List<String>,
                     override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class Button(val id: String, val label: String,
                      override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class Radio(val id: String, val label: String, val value: Boolean,
                     override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class Checkbox(val id: String, val label: String, val value: Boolean,
                        override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class SliderInt(val id: String, val label: String, val value: Int, val min: Int, val max: Int, val fmt: String,
                         override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class SliderFloat(val id: String, val label: String, val value: Float, val min: Float, val max: Float,
                           val step: Float, val fmt: String, val decimals: Int,
                           override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class InputInt(val id: String, val label: String, val value: Int, val step: Int,
                        override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class InputText(val id: String, val label: String, val value: String, val maxLen: Int,
                         override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class Text(val value: String, val color: Long?,
                    override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
    data class Header(val value: String,
                      override val disabled: Boolean, override val sameLine: Boolean, override val title: String?) : SmWidget
}

/** One user interaction, sent back to the module as an SmGui "diff". */
data class SmAction(val id: String, val type: Int, val b: Boolean = false, val i: Int = 0, val f: Float = 0f, val s: String = "")

object SmGui {
    // SmGui::DrawListElemType
    const val TYPE_BOOL = 1
    const val TYPE_INT = 2
    const val TYPE_FLOAT = 3
    const val TYPE_STRING = 4

    fun click(id: String) = SmAction(id, TYPE_BOOL, b = true)
    fun bool(id: String, v: Boolean) = SmAction(id, TYPE_BOOL, b = v)
    fun int(id: String, v: Int) = SmAction(id, TYPE_INT, i = v)
    fun float(id: String, v: Float) = SmAction(id, TYPE_FLOAT, f = v)
    fun text(id: String, v: String) = SmAction(id, TYPE_STRING, s = v)

    private val json = Json { ignoreUnknownKeys = true }

    fun parse(text: String): List<SmWidget> {
        val arr = runCatching { json.parseToJsonElement(text) as? JsonArray }.getOrNull() ?: return emptyList()
        return arr.mapNotNull { el -> runCatching { widget(el.jsonObject) }.getOrNull() }
    }

    private fun widget(o: JsonObject): SmWidget? {
        fun s(k: String) = o[k]?.jsonPrimitive?.content ?: ""
        fun i(k: String) = o[k]?.jsonPrimitive?.int ?: 0
        fun f(k: String) = o[k]?.jsonPrimitive?.float ?: 0f
        fun b(k: String) = o[k]?.jsonPrimitive?.booleanOrNull ?: false
        val dis = b("disabled")
        val same = b("sameLine")
        val title = o["title"]?.jsonPrimitive?.content?.takeIf { it.isNotBlank() }
        return when (s("t")) {
            "combo" -> SmWidget.Combo(s("id"), s("label"), i("value"),
                o["items"]?.jsonArray?.map { it.jsonPrimitive.content } ?: emptyList(), dis, same, title)
            "button" -> SmWidget.Button(s("id"), s("label"), dis, same, title)
            "radio" -> SmWidget.Radio(s("id"), s("label"), o["value"]!!.jsonPrimitive.boolean, dis, same, title)
            "checkbox" -> SmWidget.Checkbox(s("id"), s("label"), o["value"]!!.jsonPrimitive.boolean, dis, same, title)
            "slider_int" -> SmWidget.SliderInt(s("id"), s("label"), i("value"), i("min"), i("max"), s("fmt"), dis, same, title)
            "slider_float" -> SmWidget.SliderFloat(s("id"), s("label"), f("value"), f("min"), f("max"), f("step"),
                s("fmt"), i("decimals"), dis, same, title)
            "input_int" -> SmWidget.InputInt(s("id"), s("label"), i("value"), i("step"), dis, same, title)
            "input_text" -> SmWidget.InputText(s("id"), s("label"), s("value"), i("maxLen"), dis, same, title)
            "text" -> SmWidget.Text(s("value"), o["color"]?.jsonArray?.let { c ->
                val (r, g, bl, a) = c.map { (it.jsonPrimitive.float * 255).toInt().coerceIn(0, 255).toLong() }
                (a shl 24) or (r shl 16) or (g shl 8) or bl
            }, dis, same, title)
            "header" -> SmWidget.Header(s("value"), dis, same, title)
            else -> null
        }
    }
}
