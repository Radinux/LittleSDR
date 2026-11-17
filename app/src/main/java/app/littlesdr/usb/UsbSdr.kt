package app.littlesdr.usb

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbManager
import android.os.Build
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import app.littlesdr.engine.Engine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** A supported USB receiver and the SDR++ source module that drives it. */
data class UsbSdrInfo(val name: String, val sourceName: String, val vid: Int, val pid: Int)

/**
 * Finds supported receivers, asks for permission, opens them and hands the file
 * descriptor to the drivers (they use libusb_wrap_sys_device under the hood).
 */
class UsbSdrManager(private val context: Context) {
    private val usb = context.getSystemService(Context.USB_SERVICE) as UsbManager
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var connection: UsbDeviceConnection? = null
    private var openDevice: UsbDevice? = null

    private val _connected = MutableStateFlow<UsbSdrInfo?>(null)
    val connected: StateFlow<UsbSdrInfo?> = _connected.asStateFlow()

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(ctx: Context, intent: Intent) {
            val device = IntentCompat.getParcelableExtra(intent, UsbManager.EXTRA_DEVICE, UsbDevice::class.java)
            when (intent.action) {
                ACTION_PERMISSION -> {
                    if (device != null && intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) open(device)
                    else Log.w(TAG, "USB permission denied")
                }
                UsbManager.ACTION_USB_DEVICE_DETACHED -> {
                    if (device != null && device.deviceName == openDevice?.deviceName) close()
                }
            }
        }
    }

    fun register() {
        val filter = IntentFilter().apply {
            addAction(ACTION_PERMISSION)
            addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
        }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
    }

    fun unregister() {
        runCatching { context.unregisterReceiver(receiver) }
    }

    /** Scans for a supported receiver (also called when the system delivers USB_DEVICE_ATTACHED). */
    fun scan(attached: UsbDevice? = null) {
        if (openDevice != null) return
        val device = attached?.takeIf { identify(it) != null }
            ?: usb.deviceList.values.firstOrNull { identify(it) != null }
            ?: return
        if (usb.hasPermission(device)) open(device) else requestPermission(device)
    }

    private fun requestPermission(device: UsbDevice) {
        val intent = Intent(ACTION_PERMISSION).setPackage(context.packageName)
        val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
        usb.requestPermission(device, PendingIntent.getBroadcast(context, 0, intent, flags))
    }

    private fun open(device: UsbDevice) {
        val info = identify(device) ?: return
        val conn = usb.openDevice(device) ?: run { Log.e(TAG, "openDevice failed"); return }
        connection = conn
        openDevice = device
        _connected.value = info
        Log.i(TAG, "Opened ${info.name} fd=${conn.fileDescriptor}")
        scope.launch {
            Engine.ready.first { it }
            Engine.attachUsb(conn.fileDescriptor, device.vendorId, device.productId, info.sourceName)
        }
    }

    private fun close() {
        scope.launch { Engine.detachUsb() }
        connection?.close()
        connection = null
        openDevice = null
        _connected.value = null
    }

    companion object {
        private const val TAG = "LittleSDR-USB"
        private const val ACTION_PERMISSION = "app.littlesdr.USB_PERMISSION"

        private val RTL_SDR = setOf(
            0x0bda to 0x2832, 0x0bda to 0x2838, 0x0413 to 0x6680, 0x0413 to 0x6f0f, 0x0458 to 0x707f,
            0x0ccd to 0x00a9, 0x0ccd to 0x00b3, 0x0ccd to 0x00b4, 0x0ccd to 0x00b5, 0x0ccd to 0x00b7,
            0x0ccd to 0x00b8, 0x0ccd to 0x00b9, 0x0ccd to 0x00c0, 0x0ccd to 0x00c6, 0x0ccd to 0x00d3,
            0x0ccd to 0x00d7, 0x0ccd to 0x00e0, 0x1554 to 0x5020, 0x15f4 to 0x0131, 0x15f4 to 0x0133,
            0x185b to 0x0620, 0x185b to 0x0650, 0x185b to 0x0680, 0x1b80 to 0xd393, 0x1b80 to 0xd394,
            0x1b80 to 0xd395, 0x1b80 to 0xd397, 0x1b80 to 0xd398, 0x1b80 to 0xd39d, 0x1b80 to 0xd3a4,
            0x1b80 to 0xd3a8, 0x1b80 to 0xd3af, 0x1b80 to 0xd3b0, 0x1d19 to 0x1101, 0x1d19 to 0x1102,
            0x1d19 to 0x1103, 0x1d19 to 0x1104, 0x1f4d to 0xa803, 0x1f4d to 0xb803, 0x1f4d to 0xc803,
            0x1f4d to 0xd286, 0x1f4d to 0xd803,
        )
        private val HACKRF = setOf(0x1d50 to 0x604b, 0x1d50 to 0x6089, 0x1d50 to 0xcc15)

        fun identify(d: UsbDevice): UsbSdrInfo? {
            val id = d.vendorId to d.productId
            return when {
                id in RTL_SDR -> UsbSdrInfo("RTL-SDR", "RTL-SDR", d.vendorId, d.productId)
                id == (0x1d50 to 0x60a1) -> UsbSdrInfo("Airspy", "Airspy", d.vendorId, d.productId)
                id == (0x03eb to 0x800c) -> UsbSdrInfo("Airspy HF+", "Airspy HF+", d.vendorId, d.productId)
                id in HACKRF -> UsbSdrInfo("HackRF", "HackRF", d.vendorId, d.productId)
                else -> null
            }
        }
    }
}
