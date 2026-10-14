package com.plugin.mt_platform

import android.app.Activity
import androidx.core.view.WindowCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SystemBarsArgs {
    var dark: Boolean = false
}

/** Window behaviour the WebView cannot control. Nothing here handles user data. */
@TauriPlugin
class PlatformPlugin(private val activity: Activity) : Plugin(activity) {
    /** `dark` is the app theme: dark theme needs light bar icons. */
    @Command
    fun setSystemBars(invoke: Invoke) {
        val args = invoke.parseArgs(SystemBarsArgs::class.java)
        activity.runOnUiThread {
            val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
            controller.isAppearanceLightStatusBars = !args.dark
            controller.isAppearanceLightNavigationBars = !args.dark
            invoke.resolve(JSObject())
        }
    }
}
