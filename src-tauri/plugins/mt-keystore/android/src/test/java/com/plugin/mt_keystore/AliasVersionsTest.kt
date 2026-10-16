package com.plugin.mt_keystore

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test

class AliasVersionsTest {
    @Test
    fun aliasNamesRoundTrip() {
        for (version in listOf(1, 2, 9, 10, 99, 255)) {
            assertEquals(version, AliasVersions.versionOf(AliasVersions.alias(version)))
        }
        assertEquals("money_tracker_db_key_wrap_v1", AliasVersions.alias(1))
    }

    @Test
    fun onlyOurOwnAliasesAreRecognised() {
        for (other in listOf(
            "money_tracker_db_key_wrap_v", "money_tracker_db_key_wrap_v0", "money_tracker_db_key_wrap_v256",
            "money_tracker_db_key_wrap_v01", "money_tracker_db_key_wrap_v+1", "money_tracker_db_key_wrap_v1x",
            "money_tracker_db_key_wrap_v-1", "other_alias_v1", "", "money_tracker_db_key_wrap_v1000",
        )) {
            assertNull(other, AliasVersions.versionOf(other))
        }
    }

    @Test
    fun versionsIgnoreForeignAliasesAndSort() {
        val aliases = listOf("zzz", "money_tracker_db_key_wrap_v3", "money_tracker_db_key_wrap_v1", "money_tracker_db_key_wrap_v3")
        assertEquals(listOf(1, 3), AliasVersions.versions(aliases))
    }

    @Test
    fun freshInstallUsesTheFirstVersionAndAReplacementNeverReusesOne() {
        assertEquals(1, AliasVersions.current(emptyList()))
        assertEquals(1, AliasVersions.next(emptyList()))
        assertEquals(3, AliasVersions.current(listOf(1, 3)))
        // Above every existing alias, even when versions are not contiguous.
        assertEquals(4, AliasVersions.next(listOf(1, 3)))
        assertEquals(2, AliasVersions.next(listOf(1)))
    }

    @Test
    fun runningOutOfVersionsIsAnErrorNotAWrapAround() {
        try {
            AliasVersions.next(listOf(255))
            fail("expected an error")
        } catch (e: IllegalStateException) {
            // expected
        }
    }

    @Test
    fun onlyOlderVersionsAreStale() {
        assertEquals(listOf(1, 2), AliasVersions.stale(listOf(1, 2, 3), 3))
        assertEquals(emptyList<Int>(), AliasVersions.stale(listOf(1, 2, 3), 1))
        // The kept version and anything newer (an unfinished replacement) are never listed.
        assertEquals(listOf(1), AliasVersions.stale(listOf(1, 2, 3), 2))
    }

    @Test
    fun aBlobNamesItsAliasAndRejectsMalformedInput() {
        val body = ByteArray(40) { it.toByte() }
        val blob = AliasVersions.seal(7, body)
        assertEquals(41, blob.size)
        val (version, opened) = AliasVersions.open(blob, 28)!!
        assertEquals(7, version)
        assertArrayEquals(body, opened)

        assertNull(AliasVersions.open(ByteArray(0), 28))
        assertNull(AliasVersions.open(ByteArray(29), 28)) // too short for a body
        assertNull(AliasVersions.open(byteArrayOf(0) + body, 28)) // version 0
        assertNull(AliasVersions.open(AliasVersions.seal(1, ByteArray(28)), 28)) // body just too small
    }

    @Test
    fun theHighBitOfTheVersionByteIsNotASign() {
        val blob = AliasVersions.seal(200, ByteArray(40))
        assertEquals(200, AliasVersions.open(blob, 28)!!.first)
    }
}
