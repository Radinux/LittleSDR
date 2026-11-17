package app.littlesdr

import android.app.Application
import android.os.BatteryManager
import app.littlesdr.data.AppStore
import app.littlesdr.engine.Engine
import app.littlesdr.usb.UsbSdrManager

class LittleSdrApp : Application() {
    lateinit var store: AppStore
        private set
    lateinit var usb: UsbSdrManager
        private set

    override fun onCreate() {
        super.onCreate()
        store = AppStore(this)
        usb = UsbSdrManager(this).also { it.register() }
        val battery = getSystemService(BatteryManager::class.java)
        Engine.batteryLevel = { battery.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY) }
        Engine.init(this)
        usb.scan()
    }
}
