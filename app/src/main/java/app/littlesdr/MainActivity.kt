package app.littlesdr

import android.Manifest
import android.content.Intent
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.core.content.IntentCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.ui.MainScreen
import app.littlesdr.ui.SettingsScreen
import app.littlesdr.ui.RadioViewModel
import app.littlesdr.ui.theme.LittleSdrTheme

class MainActivity : ComponentActivity() {
    private val vm: RadioViewModel by viewModels()

    private val notifPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Landscape receiver: give the spectrum the full height, status bar on swipe
        WindowCompat.getInsetsController(window, window.decorView).apply {
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            hide(WindowInsetsCompat.Type.statusBars())
        }
        handleUsbIntent(intent)
        if (Build.VERSION.SDK_INT >= 33) notifPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        setContent {
            val data by vm.data.collectAsStateWithLifecycle()
            LittleSdrTheme(data.appearance) {
                var settings by rememberSaveable { mutableStateOf(false) }
                BackHandler(enabled = settings) { settings = false }
                AnimatedContent(
                    targetState = settings,
                    transitionSpec = { fadeIn() togetherWith fadeOut() },
                    label = "screen",
                ) { showSettings ->
                    if (showSettings) SettingsScreen(vm, onBack = { settings = false })
                    else MainScreen(vm, onOpenSettings = { settings = true })
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleUsbIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        (application as LittleSdrApp).usb.scan()
    }

    private fun handleUsbIntent(intent: Intent?) {
        if (intent?.action != UsbManager.ACTION_USB_DEVICE_ATTACHED) return
        val device = IntentCompat.getParcelableExtra(intent, UsbManager.EXTRA_DEVICE, UsbDevice::class.java)
        (application as LittleSdrApp).usb.scan(device)
    }
}
