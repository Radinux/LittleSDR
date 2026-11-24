package app.littlesdr.engine

import android.view.Surface
import java.net.HttpURLConnection
import java.net.URL

/**
 * JNI surface of the native engine (Rust: engine/src/android/jni.rs).
 * Every call that touches the engine must go through [Engine], which serializes them.
 */
object NativeBridge {
    init {
        System.loadLibrary("littlesdr")
    }

    // lifecycle
    @JvmStatic external fun nativeInit(rootDir: String, cacheDir: String, fftSize: Int): Boolean
    @JvmStatic external fun setUsbDevice(fd: Int, vid: Int, pid: Int)
    @JvmStatic external fun clearUsbDevice()

    // sources
    @JvmStatic external fun sourceNames(): Array<String>
    @JvmStatic external fun selectedSource(): String
    @JvmStatic external fun selectSource(name: String)
    /** Why the source stopped by itself (server refusal, unplugged...), empty if it did not */
    @JvmStatic external fun sourceError(): String
    @JvmStatic external fun sourceMenu(diffId: String, type: Int, b: Boolean, i: Int, f: Float, s: String): String

    // playback / tuning
    @JvmStatic external fun start(): Boolean
    @JvmStatic external fun stop()
    @JvmStatic external fun tune(hz: Double)
    @JvmStatic external fun tuneCenter(hz: Double)
    @JvmStatic external fun state(): DoubleArray

    // demodulator
    @JvmStatic external fun setMode(mode: Int)
    @JvmStatic external fun setBandwidth(hz: Float)
    @JvmStatic external fun setSquelch(enabled: Boolean, levelDb: Float)
    @JvmStatic external fun setVolume(volume: Float)
    @JvmStatic external fun estimateBandwidth(minBw: Double, maxBw: Double): Float
    @JvmStatic external fun options(): String
    @JvmStatic external fun setOption(key: String, value: Double)
    @JvmStatic external fun setOptionString(key: String, value: String)
    @JvmStatic external fun ft8Json(): String
    @JvmStatic external fun ft8Set(key: String, value: Double)
    @JvmStatic external fun kiwiSetSite(hostPort: String, location: String)
    @JvmStatic external fun cwJson(): String
    @JvmStatic external fun cwSet(key: String, value: Double)
    @JvmStatic external fun recJson(): String
    @JvmStatic external fun recSet(key: String, value: Double)
    @JvmStatic external fun sstvJson(): String
    @JvmStatic external fun sstvSet(key: String, value: Double)
    /** [width, height, ARGB pixels...] of the current (0) or last finished (1) SSTV image */
    @JvmStatic external fun sstvImage(which: Int): IntArray
    @JvmStatic external fun ctyLatLon(calls: Array<String>): DoubleArray
    @JvmStatic external fun rdsJson(): String

    // digital radio (DAB+ / DRM) and Meteor LRPT
    @JvmStatic external fun digitalJson(): String
    @JvmStatic external fun dabSelect(what: String)
    @JvmStatic external fun dabSlide(): ByteArray
    @JvmStatic external fun lrptJson(): String
    @JvmStatic external fun lrptSet(key: String, value: Double)
    @JvmStatic external fun lrptImage(): IntArray
    @JvmStatic external fun lrptSave(): String
    @JvmStatic external fun adsbJson(): String
    @JvmStatic external fun adsbSet(key: String, value: Double)

    // display (Vulkan renderer)
    @JvmStatic external fun setView(viewBandwidth: Double, viewOffset: Double)
    @JvmStatic external fun setDisplay(
        dbMin: Float, dbMax: Float, spectrumFraction: Float,
        fillArgb: Int, lineArgb: Int, backgroundArgb: Int, lineWidthPx: Float, palette: Int,
    )
    @JvmStatic external fun surfaceAttach(surface: Surface)
    @JvmStatic external fun surfaceResize(width: Int, height: Int)
    @JvmStatic external fun surfaceDetach()

    // ---- called from native code (headless_backend.cpp)

    @JvmStatic
    fun httpGet(url: String): String = try {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 5000
        c.readTimeout = 10000
        c.inputStream.bufferedReader().use { it.readText() }.also { c.disconnect() }
    } catch (e: Exception) {
        ""
    }

    @JvmStatic
    fun batteryLevel(): String = Engine.batteryLevel?.invoke()?.toString() ?: ""
}
