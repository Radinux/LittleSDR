package app.littlesdr.data

import android.content.Context
import android.media.MediaScannerConnection
import android.os.Environment
import java.io.File

/**
 * Files the user keeps (SSTV images, audio and baseband recordings), in a folder of the phone's
 * shared storage: Documents/LittleSDR/{SSTV,Audio,Baseband}, visible from a file manager or over USB.
 * Android 11+ lets an app create its own folder there without permission; Android 9-10 need
 * WRITE_EXTERNAL_STORAGE (asked at start). Falls back to the app's own storage if not writable.
 */
object Storage {
    const val SSTV = "SSTV"
    const val AUDIO = "Audio"
    const val BASEBAND = "Baseband"

    @Volatile private var root: File? = null

    fun root(context: Context): File = root ?: synchronized(this) {
        root ?: pick(context).also { root = it }
    }

    fun dir(context: Context, sub: String): File = File(root(context), sub).apply { mkdirs() }

    /** Creates the folders (app start, and again once the storage permission is granted). */
    fun init(context: Context) {
        root = null
        listOf(SSTV, AUDIO, BASEBAND).forEach { dir(context, it) }
    }

    /** Makes a new file show up in the gallery / music apps and over USB (MTP) right away. */
    fun scan(context: Context, f: File) {
        MediaScannerConnection.scanFile(context, arrayOf(f.path), null, null)
    }

    private fun pick(context: Context): File {
        val shared = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOCUMENTS), "LittleSDR")
        if (writable(shared)) return shared
        return File(context.getExternalFilesDir(null), "LittleSDR").apply { mkdirs() }
    }

    private fun writable(dir: File): Boolean = runCatching {
        dir.mkdirs()
        val probe = File(dir, ".probe")
        probe.writeText("")
        probe.delete()
        true
    }.getOrDefault(false)
}
