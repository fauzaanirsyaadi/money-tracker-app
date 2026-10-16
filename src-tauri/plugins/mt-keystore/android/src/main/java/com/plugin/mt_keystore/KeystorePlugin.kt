package com.plugin.mt_keystore

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class DataArgs {
    var data: String = ""
}

/**
 * Wraps and unwraps the database key with a non-exportable Android Keystore key.
 * No payload or key bytes are ever logged.
 */
@TauriPlugin
class KeystorePlugin(activity: Activity) : Plugin(activity) {
    private val vault = KeyVault()

    @Command
    fun wrap(invoke: Invoke) {
        respond(invoke) { vault.wrap(it) }
    }

    @Command
    fun unwrap(invoke: Invoke) {
        respond(invoke) { vault.unwrap(it) }
    }

    @Command
    fun securityLevel(invoke: Invoke) {
        try {
            val ret = JSObject()
            ret.put("securityLevel", vault.securityLevel())
            invoke.resolve(ret)
        } catch (e: Exception) {
            invoke.reject("keystore")
        }
    }

    @Command
    fun reset(invoke: Invoke) {
        try {
            vault.reset()
            invoke.resolve(JSObject())
        } catch (e: Exception) {
            invoke.reject("keystore")
        }
    }

    private fun respond(invoke: Invoke, op: (ByteArray) -> ByteArray) {
        try {
            val args = invoke.parseArgs(DataArgs::class.java)
            val out = op(Hex.decode(args.data))
            val ret = JSObject()
            ret.put("data", Hex.encode(out))
            invoke.resolve(ret)
        } catch (e: KeyGoneException) {
            // Permanent loss, the one case the app may offer recovery for.
            invoke.reject("keystore", "key_gone")
        } catch (e: Exception) {
            // Deliberately generic: exception text could echo sensitive state.
            invoke.reject("keystore")
        }
    }
}
