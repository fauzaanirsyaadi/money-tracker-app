package com.plugin.mt_platform

import android.app.Activity
import android.content.Intent
import androidx.activity.result.ActivityResult
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

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

@InvokeArg
class SaveArgs {
    var source: String = ""
    var name: String = ""
}

@InvokeArg
class PickArgs {
    var destination: String = ""
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

    // --- Storage Access Framework ------------------------------------------
    //
    // The bytes that pass through here are an already-encrypted backup. Paths
    // come from Rust only (this plugin has no JS-facing commands) and are still
    // required to be inside the app's cache directory.

    private fun insideCache(path: String): File? {
        val file = File(path).canonicalFile
        val cache = activity.cacheDir.canonicalFile
        return if (file.path.startsWith(cache.path + File.separator)) file else null
    }

    @Command
    fun saveFile(invoke: Invoke) {
        val args = invoke.parseArgs(SaveArgs::class.java)
        if (insideCache(args.source) == null) {
            invoke.reject("path")
            return
        }
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "application/octet-stream"
            putExtra(Intent.EXTRA_TITLE, args.name)
        }
        startActivityForResult(invoke, intent, "saveFileResult")
    }

    @ActivityCallback
    fun saveFileResult(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            reply(invoke, "cancelled")
            return
        }
        // Only a content:// URI of another app: never file:// or one of our own.
        if (!SafUri.isAcceptable(uri.scheme, uri.authority, activity.packageName)) {
            invoke.reject("uri")
            return
        }
        val args = invoke.parseArgs(SaveArgs::class.java)
        val source = insideCache(args.source)
        if (source == null) {
            invoke.reject("path")
            return
        }
        // Copy off the main thread: a large backup must not block the UI.
        Thread {
            try {
                val out = activity.contentResolver.openOutputStream(uri, "wt")
                    ?: throw IllegalStateException("no stream")
                out.use { sink -> source.inputStream().use { it.copyTo(sink) } }
                reply(invoke, "done")
            } catch (e: Exception) {
                invoke.reject("io")
            }
        }.start()
    }

    @Command
    fun pickFile(invoke: Invoke) {
        val args = invoke.parseArgs(PickArgs::class.java)
        if (insideCache(args.destination) == null) {
            invoke.reject("path")
            return
        }
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(invoke, intent, "pickFileResult")
    }

    @ActivityCallback
    fun pickFileResult(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            reply(invoke, "cancelled")
            return
        }
        if (!SafUri.isAcceptable(uri.scheme, uri.authority, activity.packageName)) {
            invoke.reject("uri")
            return
        }
        val args = invoke.parseArgs(PickArgs::class.java)
        val destination = insideCache(args.destination)
        if (destination == null) {
            invoke.reject("path")
            return
        }
        Thread {
            try {
                val input = activity.contentResolver.openInputStream(uri)
                    ?: throw IllegalStateException("no stream")
                input.use { source ->
                    destination.outputStream().use { sink ->
                        // A backup of a very large database is still far below this.
                        var total = 0L
                        val buffer = ByteArray(64 * 1024)
                        while (true) {
                            val n = source.read(buffer)
                            if (n < 0) break
                            total += n
                            if (total > MAX_BACKUP_BYTES) throw IllegalStateException("too large")
                            sink.write(buffer, 0, n)
                        }
                    }
                }
                reply(invoke, "done")
            } catch (e: Exception) {
                destination.delete()
                invoke.reject("io")
            }
        }.start()
    }

    private companion object {
        const val MAX_BACKUP_BYTES = 1L shl 30
    }
}
