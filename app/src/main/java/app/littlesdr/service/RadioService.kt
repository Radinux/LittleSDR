package app.littlesdr.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import app.littlesdr.MainActivity
import app.littlesdr.R
import app.littlesdr.engine.Engine
import app.littlesdr.ui.formatFrequency
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.dropWhile
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** Keeps the process (DSP, USB streaming, audio) alive while listening with the screen off. */
class RadioService : LifecycleService() {
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, "Réception", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Écoute en cours"
                setShowBadge(false)
            }
        )
        ServiceCompat.startForeground(
            this, NOTIF_ID, buildNotification(Engine.state.value.vfoFrequency, Engine.state.value.mode?.label),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
        )
        wakeLock = getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "LittleSDR:rx").apply { acquire(12 * 60 * 60 * 1000L) }

        lifecycleScope.launch {
            Engine.state.map { it.vfoFrequency to it.mode?.label }.distinctUntilChanged().collect { (f, m) ->
                nm.notify(NOTIF_ID, buildNotification(f, m))
            }
        }
        lifecycleScope.launch {
            // Wait until playback is seen running, then stop the service when it stops
            Engine.state.map { it.playing }.distinctUntilChanged().dropWhile { !it }.collect { playing ->
                if (!playing) stopSelf()
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_STOP) {
            lifecycleScope.launch {
                Engine.stop()
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    /** Swiped away from recents while playing: stop everything and leave RAM. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        super.onTaskRemoved(rootIntent)
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        Engine.shutdownAndExit()
    }

    override fun onDestroy() {
        wakeLock?.takeIf { it.isHeld }?.release()
        super.onDestroy()
    }

    private fun buildNotification(freq: Double, mode: String?): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, RadioService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_radio)
            .setContentTitle("${formatFrequency(freq)} MHz")
            .setContentText(mode ?: "Réception")
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, "Stop", stop)
            .build()
    }

    companion object {
        private const val CHANNEL = "rx"
        private const val NOTIF_ID = 1
        private const val ACTION_STOP = "app.littlesdr.STOP"

        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, RadioService::class.java))
        }
    }
}
