package com.plugin.mt_keystore

import android.security.keystore.KeyPermanentlyInvalidatedException
import java.security.GeneralSecurityException
import javax.crypto.AEADBadTagException
import javax.crypto.SecretKey

/**
 * The wrapping key is permanently unusable: the alias is gone, the key was
 * invalidated, or the blob does not authenticate under it. Anything else
 * (service busy, device locked) is a different, possibly temporary, failure.
 */
class KeyGoneException : Exception()

enum class KeyFailure {
    /** An explicit permanent signal: the app may offer recovery. */
    KEY_GONE,

    /** Anything else. Nothing is created or deleted because of it; the user retries. */
    TRANSIENT,
}

/**
 * Unknown or unclassified failures are transient. Only two signals are
 * permanent: [KeyGoneException] (alias missing, damaged blob, or a tag that
 * failed twice) and [KeyPermanentlyInvalidatedException].
 *
 * In particular `UnrecoverableKeyException` is **not** permanent: the framework
 * throws it for almost any Keystore error other than "no such key", including a
 * busy or restarting Keystore service.
 */
fun classify(failure: Throwable): KeyFailure =
    when (failure) {
        is KeyGoneException -> KeyFailure.KEY_GONE
        is KeyPermanentlyInvalidatedException -> KeyFailure.KEY_GONE
        else -> KeyFailure.TRANSIENT
    }

/**
 * The key, or [KeyGoneException] when there is none. Any exception thrown by
 * [load] is passed on unchanged, so it is classified like every other failure.
 */
fun keyOrGone(load: () -> SecretKey?): SecretKey = load() ?: throw KeyGoneException()

/**
 * Runs [attempt] and, if the GCM tag does not verify, once more with a fresh
 * attempt. A tag that fails twice is a blob that does not belong to the key; a
 * single failure can be a hiccup of the Keystore service, which is not grounds
 * for offering to delete data.
 */
fun <T> authenticatedTwice(attempt: () -> T): T =
    try {
        attempt()
    } catch (first: AEADBadTagException) {
        try {
            attempt()
        } catch (second: AEADBadTagException) {
            throw KeyGoneException()
        }
    }

/** Keeps the compiler honest that every checked failure above is a security exception. */
@Suppress("unused")
private fun marker(e: GeneralSecurityException) = e
