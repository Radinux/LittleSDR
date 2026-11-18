package app.littlesdr.ui.components

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.core.content.FileProvider
import java.io.File

/** Opens / shares a recording or an SSTV image through the app's FileProvider. */
object FileActions {
    private fun uri(context: Context, f: File) = FileProvider.getUriForFile(context, "${context.packageName}.files", f)

    fun mime(f: File) = when (f.extension.lowercase()) {
        "png" -> "image/png"
        "wav" -> "audio/wav"
        else -> "application/octet-stream"
    }

    fun open(context: Context, f: File) {
        val i = Intent(Intent.ACTION_VIEW).setDataAndType(uri(context, f), mime(f)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        try {
            context.startActivity(i)
        } catch (_: ActivityNotFoundException) {
            Toast.makeText(context, "Aucune application pour ouvrir ce fichier", Toast.LENGTH_SHORT).show()
        }
    }

    fun share(context: Context, f: File) {
        val i = Intent(Intent.ACTION_SEND).setType(mime(f))
            .putExtra(Intent.EXTRA_STREAM, uri(context, f))
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        context.startActivity(Intent.createChooser(i, f.name))
    }
}

/** "12,3 Mo" style size. */
fun formatBytes(b: Long): String = when {
    b >= 1L shl 30 -> String.format(java.util.Locale.FRANCE, "%.1f Go", b / 1073741824.0)
    b >= 1L shl 20 -> String.format(java.util.Locale.FRANCE, "%.1f Mo", b / 1048576.0)
    b >= 1L shl 10 -> "${b shr 10} ko"
    else -> "$b o"
}
