package com.plugin.mt_keystore

import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.UserNotAuthenticatedException
import java.io.IOException
import java.security.InvalidAlgorithmParameterException
import java.security.InvalidKeyException
import java.security.KeyStoreException
import java.security.ProviderException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import javax.crypto.IllegalBlockSizeException
import javax.crypto.SecretKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.fail
import org.junit.Test

class KeyFailuresTest {
    // --- classification: only explicit permanent signals are KEY_GONE ----------

    @Test
    fun explicitPermanentSignalsAreKeyGone() {
        assertEquals(KeyFailure.KEY_GONE, classify(KeyGoneException()))
        assertEquals(KeyFailure.KEY_GONE, classify(KeyPermanentlyInvalidatedException()))
    }

    @Test
    fun unrecoverableKeyExceptionIsTransient() {
        // The framework throws it for any Keystore error except "no such key",
        // including a busy or restarting service.
        assertEquals(KeyFailure.TRANSIENT, classify(UnrecoverableKeyException("Failed to obtain information about key")))
    }

    @Test
    fun everyOtherKeystoreExceptionIsTransient() {
        val others = listOf<Throwable>(
            UserNotAuthenticatedException(), // device locked
            KeyStoreException("keystore busy"),
            ProviderException("system error"),
            IllegalBlockSizeException("bad size"),
            InvalidAlgorithmParameterException("bad parameter"),
            InvalidKeyException("generic invalid key"),
            IOException("io"),
            IllegalStateException("state"),
            RuntimeException("unclassified"),
            Exception("unknown"),
            // A raw tag failure is decided by authenticatedTwice, not by classify.
            AEADBadTagException("tag"),
        )
        for (failure in others) {
            assertEquals("${failure::class.java.simpleName} must be transient", KeyFailure.TRANSIENT, classify(failure))
        }
    }

    @Test
    fun anUnknownThrowableIsTransient() {
        assertEquals(KeyFailure.TRANSIENT, classify(Error("whatever")))
    }

    // --- missing alias ---------------------------------------------------------

    @Test
    fun aMissingKeyIsGone() {
        try {
            keyOrGone { null }
            fail("expected KeyGoneException")
        } catch (e: KeyGoneException) {
            // expected
        }
    }

    @Test
    fun aKeystoreErrorWhileLoadingIsPassedOnNotTurnedIntoGone() {
        val failure = UnrecoverableKeyException("busy")
        try {
            keyOrGone { throw failure }
            fail("expected the exception to propagate")
        } catch (e: UnrecoverableKeyException) {
            assertSame(failure, e)
            assertEquals(KeyFailure.TRANSIENT, classify(e))
        }
    }

    // --- tag failures: once is a hiccup, twice is a blob that does not belong --

    @Test
    fun aSingleTagFailureIsRetriedAndSucceeds() {
        var calls = 0
        val result = authenticatedTwice {
            calls += 1
            if (calls == 1) throw AEADBadTagException("first")
            "plain"
        }
        assertEquals("plain", result)
        assertEquals(2, calls)
    }

    @Test
    fun aTagThatFailsTwiceIsGone() {
        var calls = 0
        try {
            authenticatedTwice<String> {
                calls += 1
                throw AEADBadTagException("again")
            }
            fail("expected KeyGoneException")
        } catch (e: KeyGoneException) {
            assertEquals(KeyFailure.KEY_GONE, classify(e))
        }
        assertEquals(2, calls)
    }

    @Test
    fun otherFailuresAreNeverRetriedAndStayTransient() {
        for (failure in listOf<Exception>(
            IllegalBlockSizeException("x"),
            KeyStoreException("x"),
            UnrecoverableKeyException("x"),
            UserNotAuthenticatedException(),
        )) {
            var calls = 0
            try {
                authenticatedTwice<String> {
                    calls += 1
                    throw failure
                }
                fail("expected ${failure::class.java.simpleName}")
            } catch (e: Exception) {
                assertSame(failure, e)
                assertEquals(KeyFailure.TRANSIENT, classify(e))
            }
            assertEquals("${failure::class.java.simpleName} must not be retried", 1, calls)
        }
    }

    @Test
    fun aPermanentlyInvalidatedKeyIsGoneWithoutRetry() {
        var calls = 0
        try {
            authenticatedTwice<String> {
                calls += 1
                throw KeyPermanentlyInvalidatedException()
            }
            fail("expected KeyPermanentlyInvalidatedException")
        } catch (e: KeyPermanentlyInvalidatedException) {
            assertEquals(KeyFailure.KEY_GONE, classify(e))
        }
        assertEquals(1, calls)
    }

    @Suppress("unused")
    private fun neverCalled(key: SecretKey) = key
}
