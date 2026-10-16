package com.moneytracker.app

import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.os.StrictMode
import android.view.View
import android.view.WindowManager
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    val debuggable = (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
    if (debuggable) {
      // Debug builds only: report (to logcat, tag StrictMode) disk and network
      // work on the main thread. It sees Java/Kotlin I/O, not native Rust I/O.
      StrictMode.setThreadPolicy(
        StrictMode.ThreadPolicy.Builder()
          .detectDiskReads()
          .detectDiskWrites()
          .detectNetwork()
          .penaltyLog()
          .build()
      )
    }

    enableEdgeToEdge()
    super.onCreate(savedInstanceState)

    // Keeps balances out of screenshots, screen recordings and the recent-apps
    // thumbnail, on every screen. Debuggable builds may opt out with
    // `am start --ez mt_insecure true` so the emulator can be captured; a
    // release build ignores the extra.
    if (!(debuggable && intent?.getBooleanExtra("mt_insecure", false) == true)) {
      window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
    }

    // Edge-to-edge draws behind the status bar, the gesture bar and the cutout.
    // Pad the content by those insets (and the keyboard, so a bottom sheet is
    // never hidden behind it) instead of relying on CSS env() support.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
