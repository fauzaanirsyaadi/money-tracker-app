package com.plugin.mt_platform

import android.security.keystore.KeyPermanentlyInvalidatedException
import java.security.GeneralSecurityException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BiometricKeyPolicyTest {
    private val fate = BiometricKeyPolicy::classify

    @Test
    fun aNewEnrolmentOrAMissingKeyInvalidatesBiometricUnlock() {
        assertEquals(BiometricKeyPolicy.Fate.INVALIDATED, fate(KeyPermanentlyInvalidatedException()))
        assertEquals(BiometricKeyPolicy.Fate.INVALIDATED, fate(BiometricKeyPolicy.KeyMissing()))
        // A sealed token that no longer authenticates belongs to a different key.
        assertEquals(BiometricKeyPolicy.Fate.INVALIDATED, fate(AEADBadTagException()))
    }

    @Test
    fun everythingElseIsTransientAndKeepsBiometricEnabled() {
        for (failure in listOf<Throwable>(
            UnrecoverableKeyException(),
            GeneralSecurityException("keystore busy"),
            IllegalStateException(),
            RuntimeException("unknown"),
            java.io.IOException(),
        )) {
            assertEquals(failure.toString(), BiometricKeyPolicy.Fate.TRANSIENT, fate(failure))
        }
    }

    /**
     * The point of F8: the biometric key's failure path shares nothing with the
     * database key's. Its alias can never be mistaken for a database alias (and
     * so is never counted, kept or pruned as one), and the vocabulary of its
     * outcomes has no "key gone" at all.
     */
    @Test
    fun theBiometricKeyIsNeverTheDatabaseKey() {
        val dbPrefix = "money_tracker_db_key_wrap_v"
        assertFalse(BiometricKeyPolicy.ALIAS.startsWith(dbPrefix))
        assertTrue(BiometricKeyPolicy.ALIAS.startsWith("money_tracker_biometric"))
        assertEquals(setOf("INVALIDATED", "TRANSIENT"), BiometricKeyPolicy.Fate.values().map { it.name }.toSet())
        assertTrue(BiometricKeyPolicy.Fate.values().none { it.name.contains("GONE") })
    }

    @Test
    fun blobsRoundTripAndShortOnesAreRejected() {
        val iv = ByteArray(BiometricKeyPolicy.IV_BYTES) { it.toByte() }
        val body = ByteArray(48) { (100 + it).toByte() }
        val (gotIv, gotBody) = BiometricKeyPolicy.splitBlob(BiometricKeyPolicy.joinBlob(iv, body))!!
        assertArrayEquals(iv, gotIv)
        assertArrayEquals(body, gotBody)
        assertNull(BiometricKeyPolicy.splitBlob(ByteArray(0)))
        assertNull(BiometricKeyPolicy.splitBlob(ByteArray(BiometricKeyPolicy.IV_BYTES + 15)))
    }

    @Test
    fun hexRoundTripsAndRejectsMalformedText() {
        val bytes = byteArrayOf(0, 1, 15, 16, 127, -128, -1)
        assertEquals("00010f107f80ff", BiometricKeyPolicy.hex(bytes))
        assertArrayEquals(bytes, BiometricKeyPolicy.unhex("00010F107F80FF"))
        assertArrayEquals(ByteArray(0), BiometricKeyPolicy.unhex(""))
        for (bad in listOf("0", "abc", "zz", "0g", "+1", " 1")) {
            assertNull(bad, BiometricKeyPolicy.unhex(bad))
        }
    }
}
