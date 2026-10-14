package com.plugin.mt_platform

import android.app.Activity
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.fragment.app.FragmentActivity
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

@InvokeArg
class PromptArgs {
    var title: String = ""
    var subtitle: String = ""
    var negative: String = ""
}

/** Window and prompt behaviour the WebView cannot do. Handles no user data. */
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

    @Command
    fun biometricStatus(invoke: Invoke) {
        val status = when (BiometricManager.from(activity).canAuthenticate(BIOMETRIC_STRONG)) {
            BiometricManager.BIOMETRIC_SUCCESS -> "available"
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "none_enrolled"
            else -> "unavailable"
        }
        val ret = JSObject()
        ret.put("status", status)
        invoke.resolve(ret)
    }

    /**
     * Shows the system prompt. The result is a plain gate on the app UI: the
     * database key is not bound to it (see docs/SECURITY.md), so changing the
     * enrolled fingerprints can never make the data unreadable.
     */
    @Command
    fun authenticate(invoke: Invoke) {
        val args = invoke.parseArgs(PromptArgs::class.java)
        val host = activity as? FragmentActivity
        if (host == null) {
            reply(invoke, "failed")
            return
        }
        var answered = false
        val answer = { outcome: String ->
            if (!answered) {
                answered = true
                reply(invoke, outcome)
            }
        }
        host.runOnUiThread {
            val prompt = BiometricPrompt(
                host,
                ContextCompat.getMainExecutor(host),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        answer("success")
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        when (errorCode) {
                            BiometricPrompt.ERROR_USER_CANCELED,
                            BiometricPrompt.ERROR_NEGATIVE_BUTTON,
                            BiometricPrompt.ERROR_CANCELED -> answer("cancelled")
                            else -> answer("failed")
                        }
                    }
                    // A single unrecognised fingerprint keeps the prompt open; nothing to do.
                },
            )
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(args.title)
                .setSubtitle(args.subtitle)
                .setNegativeButtonText(args.negative)
                .setAllowedAuthenticators(BIOMETRIC_STRONG)
                .setConfirmationRequired(false)
                .build()
            prompt.authenticate(info)
        }
    }

    private fun reply(invoke: Invoke, outcome: String) {
        val ret = JSObject()
        ret.put("outcome", outcome)
        invoke.resolve(ret)
    }
}
