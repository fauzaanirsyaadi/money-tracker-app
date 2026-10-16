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

@InvokeArg
class VersionArgs {
    var version: Int = 0
}

@InvokeArg
class VersionDataArgs {
    var version: Int = 0
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
    fun nextAliasVersion(invoke: Invoke) {
        guard(invoke) {
            val ret = JSObject()
            ret.put("version", vault.nextAliasVersion())
            invoke.resolve(ret)
        }
    }

    @Command
    fun wrapNewAlias(invoke: Invoke) {
        guard(invoke) {
            val args = invoke.parseArgs(VersionDataArgs::class.java)
            val ret = JSObject()
            ret.put("data", Hex.encode(vault.wrapWithNewAlias(args.version, Hex.decode(args.data))))
            invoke.resolve(ret)
        }
    }

    @Command
    fun deleteAlias(invoke: Invoke) {
        guard(invoke) {
            vault.deleteAlias(invoke.parseArgs(VersionArgs::class.java).version)
            invoke.resolve(JSObject())
        }
    }

    @Command
    fun pruneBelow(invoke: Invoke) {
        guard(invoke) {
            vault.pruneBelow(invoke.parseArgs(VersionArgs::class.java).version)
            invoke.resolve(JSObject())
        }
    }

    /** Runs `work`; any failure is classified like every other Keystore failure. */
    private fun guard(invoke: Invoke, work: () -> Unit) {
        try {
            work()
        } catch (e: Exception) {
            when (classify(e)) {
                KeyFailure.KEY_GONE -> invoke.reject("keystore", "key_gone")
                KeyFailure.TRANSIENT -> invoke.reject("keystore")
            }
        }
    }

    private fun respond(invoke: Invoke, op: (ByteArray) -> ByteArray) {
        try {
            val args = invoke.parseArgs(DataArgs::class.java)
            val out = op(Hex.decode(args.data))
            val ret = JSObject()
            ret.put("data", Hex.encode(out))
            invoke.resolve(ret)
        } catch (e: Exception) {
            // Only an explicit permanent signal may lead to the recovery screen;
            // everything else is "try again". The message stays generic:
            // exception text could echo sensitive state.
            when (classify(e)) {
                KeyFailure.KEY_GONE -> invoke.reject("keystore", "key_gone")
                KeyFailure.TRANSIENT -> invoke.reject("keystore")
            }
        }
    }
}
