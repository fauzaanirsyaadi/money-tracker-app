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

/**
 * AES-256-GCM keys held in the Android Keystore under versioned aliases
 * ([AliasVersions]); output layout is `version(1) || iv(12) || ciphertext+tag`.
 */
class KeyVault {
    private val keyStore: KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun aliasVersions(): List<Int> = AliasVersions.versions(keyStore.aliases().toList())

    /**
     * The existing key of that version, or null when it does not exist. Never
     * creates one. Any exception (including `UnrecoverableKeyException`) is
     * passed on as it is: see [classify] for which ones count as permanent.
     */
    private fun existingKey(version: Int): SecretKey? =
        keyStore.getKey(AliasVersions.alias(version), null) as? SecretKey

    /** Creates the key of that version. Callers check it does not exist. */
    @Synchronized
    private fun createKey(version: Int): SecretKey {
        // StrongBox first; many devices and emulators lack it, so fall back to the TEE.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            try {
                return generate(version, strongBox = true)
            } catch (e: Exception) {
                // fall through to the default provider
            }
        }
        return generate(version, strongBox = false)
    }

    private fun generate(version: Int, strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            AliasVersions.alias(version),
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

    private fun encrypt(version: Int, key: SecretKey, plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key)
        return AliasVersions.seal(version, cipher.iv + cipher.doFinal(plain))
    }

    /**
     * Wraps with the newest alias, creating the first one on a fresh install.
     * This is the only place besides [wrapWithNewAlias] that creates a key.
     */
    @Synchronized
    fun wrap(plain: ByteArray): ByteArray {
        val version = AliasVersions.current(aliasVersions())
        val key = existingKey(version) ?: createKey(version)
        return encrypt(version, key, plain)
    }

    /** The version a replacement key will get: one above every existing alias. */
    fun nextAliasVersion(): Int = AliasVersions.next(aliasVersions())

    /**
     * Creates the alias `version` and wraps with it, leaving every older alias
     * untouched. Refuses a version that already exists.
     */
    @Synchronized
    fun wrapWithNewAlias(version: Int, plain: ByteArray): ByteArray {
        check(version !in aliasVersions()) { "alias already exists" }
        return encrypt(version, createKey(version), plain)
    }

    /**
     * Never creates a key: if none exists the data it protected is already
     * unreadable, and a new key would only turn that into silent data loss.
     * The blob names the alias that wrapped it.
     */
    fun unwrap(blob: ByteArray): ByteArray {
        val (version, body) = AliasVersions.open(blob, IV_BYTES + TAG_BYTES) ?: throw KeyGoneException()
        val key = keyOrGone { existingKey(version) }
        return authenticatedTwice {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BYTES * 8, body, 0, IV_BYTES))
            cipher.doFinal(body, IV_BYTES, body.size - IV_BYTES)
        }
    }

    /** Deletes one alias. Called only for an unused new alias, or after the new data is verified. */
    @Synchronized
    fun deleteAlias(version: Int) {
        keyStore.deleteEntry(AliasVersions.alias(version))
    }

    /** Deletes every alias older than `keep`, which must be the one the verified data uses. */
    @Synchronized
    fun pruneBelow(keep: Int) {
        for (version in AliasVersions.stale(aliasVersions(), keep)) {
            keyStore.deleteEntry(AliasVersions.alias(version))
        }
    }

    /** Reports the level of the existing key. Never creates one. */
    fun securityLevel(): String {
        val key = keyOrGone { existingKey(AliasVersions.current(aliasVersions())) }
        val info = SecretKeyFactory.getInstance(key.algorithm, "AndroidKeyStore")
            .getKeySpec(key, KeyInfo::class.java) as KeyInfo
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
