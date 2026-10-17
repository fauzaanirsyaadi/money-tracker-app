package com.plugin.mt_platform

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
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
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

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
class UnlockArgs {
    var title: String = ""
    var subtitle: String = ""
    var negative: String = ""
    var blob: String = ""
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

    // --- Biometric unlock ---------------------------------------------------
    //
    // A key of its own (BiometricKeyPolicy.ALIAS, never the database key) is
    // bound to the prompt through a CryptoObject, so the prompt's success is not
    // just a boolean: the key opens only after a real fingerprint or face, and
    // only the token it releases unlocks the app. The key is invalidated when a
    // new fingerprint or face is enrolled; that switches biometric unlock off
    // and is never a storage problem.

    private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun newBiometricKey(): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            BiometricKeyPolicy.ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(true)
            .setInvalidatedByBiometricEnrollment(true)
            .apply {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    // Every use needs a fresh strong biometric, not a recent unlock.
                    setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                } else {
                    @Suppress("DEPRECATION")
                    setUserAuthenticationValidityDurationSeconds(-1)
                }
            }
            .build()
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(spec)
        return generator.generateKey()
    }

    private fun existingBiometricKey(): SecretKey =
        (keyStore().getKey(BiometricKeyPolicy.ALIAS, null) as? SecretKey) ?: throw BiometricKeyPolicy.KeyMissing()

    /** Removes the biometric key, if there is one. Never touches any other alias. */
    private fun forgetBiometricKey() {
        val store = keyStore()
        if (store.containsAlias(BiometricKeyPolicy.ALIAS)) store.deleteEntry(BiometricKeyPolicy.ALIAS)
    }

    private fun replyBiometric(invoke: Invoke, outcome: String, token: ByteArray? = null, blob: ByteArray? = null) {
        val ret = JSObject()
        ret.put("outcome", outcome)
        if (token != null) ret.put("token", BiometricKeyPolicy.hex(token))
        if (blob != null) ret.put("blob", BiometricKeyPolicy.hex(blob))
        invoke.resolve(ret)
    }

    /**
     * Shows the prompt with [cipher] bound to it. [onSuccess] receives the
     * cipher the system authorised and must produce the reply.
     */
    private fun promptWith(
        invoke: Invoke,
        args: PromptArgs,
        cipher: Cipher,
        onSuccess: (Cipher) -> Unit,
    ) {
        val host = activity as? FragmentActivity
        if (host == null) {
            replyBiometric(invoke, "failed")
            return
        }
        var answered = false
        val answerOnce = { block: () -> Unit ->
            if (!answered) {
                answered = true
                block()
            }
        }
        host.runOnUiThread {
            val prompt = BiometricPrompt(
                host,
                ContextCompat.getMainExecutor(host),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        answerOnce {
                            val authorised = result.cryptoObject?.cipher
                            if (authorised == null) {
                                replyBiometric(invoke, "failed")
                            } else {
                                try {
                                    onSuccess(authorised)
                                } catch (e: Exception) {
                                    replyBiometric(invoke, outcomeOf(e))
                                }
                            }
                        }
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        answerOnce {
                            when (errorCode) {
                                BiometricPrompt.ERROR_USER_CANCELED,
                                BiometricPrompt.ERROR_NEGATIVE_BUTTON,
                                BiometricPrompt.ERROR_CANCELED -> replyBiometric(invoke, "cancelled")
                                else -> replyBiometric(invoke, "failed")
                            }
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
            prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
        }
    }

    private fun outcomeOf(failure: Throwable): String =
        when (BiometricKeyPolicy.classify(failure)) {
            BiometricKeyPolicy.Fate.INVALIDATED -> "invalidated"
            BiometricKeyPolicy.Fate.TRANSIENT -> "failed"
        }

    /**
     * Creates a fresh biometric key, proves it with the user's fingerprint or
     * face, and returns a random token together with the token encrypted under
     * that key. Rust keeps the blob and a hash of the token; the token itself
     * is not stored anywhere.
     */
    @Command
    fun biometricEnroll(invoke: Invoke) {
        val args = invoke.parseArgs(PromptArgs::class.java)
        val cipher: Cipher
        try {
            forgetBiometricKey()
            val key = newBiometricKey()
            cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, key)
        } catch (e: Exception) {
            replyBiometric(invoke, outcomeOf(e))
            return
        }
        promptWith(invoke, args, cipher) { authorised ->
            val token = ByteArray(BiometricKeyPolicy.TOKEN_BYTES).also { SecureRandom().nextBytes(it) }
            val sealed = authorised.doFinal(token)
            replyBiometric(invoke, "success", token, BiometricKeyPolicy.joinBlob(authorised.iv, sealed))
        }
    }

    /**
     * Opens the token sealed at enrolment. Only a successful strong biometric
     * lets the key decrypt it. A key invalidated by a new enrolment (or missing)
     * answers "invalidated"; the app then switches biometric unlock off.
     */
    @Command
    fun biometricUnlock(invoke: Invoke) {
        val args = invoke.parseArgs(UnlockArgs::class.java)
        val split = BiometricKeyPolicy.unhex(args.blob)?.let { BiometricKeyPolicy.splitBlob(it) }
        if (split == null) {
            replyBiometric(invoke, "invalidated")
            return
        }
        val (iv, sealed) = split
        val cipher: Cipher
        try {
            cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, existingBiometricKey(), GCMParameterSpec(128, iv))
        } catch (e: Exception) {
            replyBiometric(invoke, outcomeOf(e))
            return
        }
        promptWith(invoke, PromptArgs().also {
            it.title = args.title
            it.subtitle = args.subtitle
            it.negative = args.negative
        }, cipher) { authorised ->
            replyBiometric(invoke, "success", token = authorised.doFinal(sealed))
        }
    }

    /** Switches biometric unlock off at the Keystore: deletes this key and only this key. */
    @Command
    fun biometricForget(invoke: Invoke) {
        try {
            forgetBiometricKey()
            replyBiometric(invoke, "done")
        } catch (e: Exception) {
            replyBiometric(invoke, "failed")
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
