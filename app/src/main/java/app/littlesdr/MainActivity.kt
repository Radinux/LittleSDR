package app.littlesdr

import android.Manifest
import android.content.Intent
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.littlesdr.engine.Engine
import app.littlesdr.ui.MainScreen
import app.littlesdr.ui.Ft8Screen
import app.littlesdr.ui.SettingsScreen
import app.littlesdr.ui.RadioViewModel
import app.littlesdr.ui.theme.LittleSdrTheme

private enum class Screen { Main, Settings, Ft8 }

class MainActivity : ComponentActivity() {
    private val vm: RadioViewModel by viewModels()

    private val notifPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}
    // Android 9-10: writing to Documents/LittleSDR needs the storage permission
    private val storagePermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) app.littlesdr.data.Storage.init(this)
    }

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
        if (Build.VERSION.SDK_INT <= 29 &&
            checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) != android.content.pm.PackageManager.PERMISSION_GRANTED
        ) storagePermission.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE)
        setContent {
            val data by vm.data.collectAsStateWithLifecycle()
            val playing by vm.state.collectAsStateWithLifecycle().let { s -> remember { derivedStateOf { s.value.playing } } }
            // Screen stays on while receiving, on every page (main, FT8, settings)
            LaunchedEffect(playing) {
                if (playing) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
            LittleSdrTheme(data.appearance) {
                var screen by rememberSaveable { mutableStateOf(Screen.Main) }
                BackHandler(enabled = screen != Screen.Main) { screen = Screen.Main }
                AnimatedContent(
                    targetState = screen,
                    transitionSpec = { fadeIn() togetherWith fadeOut() },
                    label = "screen",
                ) { s ->
                    when (s) {
                        Screen.Main -> MainScreen(vm, onOpenSettings = { screen = Screen.Settings }, onOpenFt8 = { screen = Screen.Ft8 })
                        Screen.Settings -> SettingsScreen(vm, onBack = { screen = Screen.Main })
                        Screen.Ft8 -> Ft8Screen(vm, onBack = { screen = Screen.Main })
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleUsbIntent(intent)
    }

    override fun onDestroy() {
        super.onDestroy()
        // Back out / swipe away while idle: really quit (native engine, USB, audio).
        // While playing, the foreground service keeps the radio going in background.
        if (isFinishing && !isChangingConfigurations && !Engine.state.value.playing) Engine.shutdownAndExit()
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
