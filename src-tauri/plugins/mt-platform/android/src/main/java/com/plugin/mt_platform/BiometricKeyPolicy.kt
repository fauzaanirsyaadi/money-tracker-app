package com.plugin.mt_platform

import android.security.keystore.KeyPermanentlyInvalidatedException
import javax.crypto.AEADBadTagException

/**
 * The key that releases the app-unlock token after a fingerprint or face.
 *
 * It is **not** the database key and has nothing to do with it: this alias is
 * never read, written or deleted by the Keystore plugin, and nothing here is
 * ever reported to the app as "key gone". A fingerprint added to the phone
 * invalidates *this* key, which only switches biometric unlock off; the PIN
 * still opens the app and the data is untouched.
 */
object BiometricKeyPolicy {
    /** Distinct from every `money_tracker_db_key_wrap_v<N>` alias. */
    const val ALIAS = "money_tracker_biometric_unlock_v1"

    const val TOKEN_BYTES = 32
    const val IV_BYTES = 12

    /** What happened to the biometric key (never about the database key). */
    enum class Fate {
        /** Enrolment changed or the key is gone: biometric unlock must be switched off. */
        INVALIDATED,

        /** Anything else: keep biometric enabled, the user can use the PIN meanwhile. */
        TRANSIENT,
    }

    /**
     * Only three signals are permanent: the framework's
     * [KeyPermanentlyInvalidatedException] (a new fingerprint or face was
     * enrolled), [KeyMissing] (the alias is no longer in the Keystore) and a
     * sealed token that does not authenticate under the key (the blob no longer
     * belongs to it). Each only switches biometric unlock off, which costs the
     * user a re-enable and nothing else. Anything unrecognised is transient, as
     * everywhere in this app.
     */
    fun classify(failure: Throwable): Fate =
        when (failure) {
            is KeyPermanentlyInvalidatedException -> Fate.INVALIDATED
            is KeyMissing -> Fate.INVALIDATED
            is AEADBadTagException -> Fate.INVALIDATED
            else -> Fate.TRANSIENT
        }

    /** The alias holds no key any more. */
    class KeyMissing : Exception()

    /** `iv || ciphertext+tag`, the blob kept by the app next to the lock. */
    fun joinBlob(iv: ByteArray, ciphertext: ByteArray): ByteArray = iv + ciphertext

    /** Splits a blob, or null when it cannot be one (too short to hold an IV and a tag). */
    fun splitBlob(blob: ByteArray): Pair<ByteArray, ByteArray>? {
        if (blob.size < IV_BYTES + 16) return null
        return blob.copyOfRange(0, IV_BYTES) to blob.copyOfRange(IV_BYTES, blob.size)
    }

    fun hex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }

    /** Decodes lower- or upper-case hex, or null for odd length or other characters. */
    fun unhex(text: String): ByteArray? {
        if (text.length % 2 != 0) return null
        val out = ByteArray(text.length / 2)
        for (i in out.indices) {
            val hi = Character.digit(text[2 * i], 16)
            val lo = Character.digit(text[2 * i + 1], 16)
            if (hi < 0 || lo < 0) return null
            out[i] = ((hi shl 4) or lo).toByte()
        }
        return out
    }
}
