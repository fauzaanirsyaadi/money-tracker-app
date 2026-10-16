package com.plugin.mt_keystore

import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec

/** AES-256-GCM key held in the Android Keystore; output layout is `iv(12) || ciphertext+tag`. */
class KeyVault {
    private val keyStore: KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    /**
     * The existing key, or null when no key exists. Never creates one. Any
     * exception (including `UnrecoverableKeyException`) is passed on as it is:
     * see [classify] for which ones count as permanent.
     */
    private fun existingKey(): SecretKey? = keyStore.getKey(ALIAS, null) as? SecretKey

    /** The existing key, creating one if none exists. Only `wrap` may create. */
    @Synchronized
    private fun key(): SecretKey {
        existingKey()?.let { return it }
        // StrongBox first; many devices and emulators lack it, so fall back to the TEE.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            try {
                return generate(strongBox = true)
            } catch (e: Exception) {
                // fall through to the default provider
            }
        }
        return generate(strongBox = false)
    }

    private fun generate(strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            // The key is unusable while the device is locked. The app has no
            // background work, so nothing needs it then. This is *not* a
            // per-use user-authentication requirement: such a key would be lost
            // when fingerprints change, and the data with it (docs/SECURITY.md).
            spec.setUnlockedDeviceRequired(true)
            if (strongBox) spec.setIsStrongBoxBacked(true)
        }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(spec.build())
        return generator.generateKey()
    }

    fun wrap(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val iv = cipher.iv
        return iv + cipher.doFinal(plain)
    }

    /**
     * Never creates a key: if none exists the data it protected is already
     * unreadable, and a new key would only turn that into silent data loss.
     */
    fun unwrap(blob: ByteArray): ByteArray {
        if (blob.size <= IV_BYTES + TAG_BYTES) throw KeyGoneException()
        val key = keyOrGone { existingKey() }
        return authenticatedTwice {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BYTES * 8, blob, 0, IV_BYTES))
            cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES)
        }
    }

    /** Deletes the wrapping key (restore over unreadable data, after confirmation). */
    @Synchronized
    fun reset() {
        keyStore.deleteEntry(ALIAS)
    }

    fun securityLevel(): String {
        val info = SecretKeyFactory.getInstance(key().algorithm, "AndroidKeyStore")
            .getKeySpec(key(), KeyInfo::class.java) as KeyInfo
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            return when (info.securityLevel) {
                KeyProperties.SECURITY_LEVEL_STRONGBOX -> "strongbox"
                KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "tee"
                KeyProperties.SECURITY_LEVEL_SOFTWARE -> "software"
                else -> "unknown"
            }
        }
        @Suppress("DEPRECATION")
        return if (info.isInsideSecureHardware) "tee" else "software"
    }

    private companion object {
        const val ALIAS = "money_tracker_db_key_wrap_v1"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BYTES = 16
    }
}

object Hex {
    private const val DIGITS = "0123456789abcdef"

    fun encode(bytes: ByteArray): String {
        val sb = StringBuilder(bytes.size * 2)
        for (b in bytes) {
            val v = b.toInt() and 0xff
            sb.append(DIGITS[v shr 4]).append(DIGITS[v and 0x0f])
        }
        return sb.toString()
    }

    fun decode(text: String): ByteArray {
        require(text.length % 2 == 0)
        return ByteArray(text.length / 2) { i ->
            val hi = DIGITS.indexOf(text[2 * i])
            val lo = DIGITS.indexOf(text[2 * i + 1])
            require(hi >= 0 && lo >= 0)
            ((hi shl 4) or lo).toByte()
        }
    }
}
