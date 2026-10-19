package com.plugin.mt_platform

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SafUriTest {
    private val own = "io.github.fauzaanirsyaadi.moneytracker"

    private fun ok(scheme: String?, authority: String?) = SafUri.isAcceptable(scheme, authority, own)

    @Test
    fun contentUrisOfOtherAppsAreAccepted() {
        for (authority in listOf(
            "com.android.providers.downloads.documents",
            "com.android.externalstorage.documents",
            "com.google.android.apps.docs.storage",
            "com.android.providers.media.documents",
        )) {
            assertTrue(authority, ok("content", authority))
        }
        // The scheme is case-insensitive.
        assertTrue(ok("CONTENT", "com.android.providers.downloads.documents"))
        assertTrue(ok("Content", "com.android.providers.downloads.documents"))
    }

    @Test
    fun fileUrisAreRejectedWhateverTheyPointAt() {
        // file:///data/user/0/io.github.fauzaanirsyaadi.moneytracker/files/db.key has no authority.
        assertFalse(ok("file", null))
        assertFalse(ok("file", ""))
        assertFalse(ok("FILE", "localhost"))
    }

    @Test
    fun everyOtherSchemeIsRejected() {
        for (scheme in listOf("http", "https", "ftp", "android.resource", "intent", "javascript", "data", "mailto", "", "content:", " content")) {
            assertFalse(scheme, ok(scheme, "com.android.providers.downloads.documents"))
        }
        assertFalse(ok(null, "com.android.providers.downloads.documents"))
    }

    @Test
    fun aContentUriWithoutAnAuthorityIsRejected() {
        assertFalse(ok("content", null))
        assertFalse(ok("content", ""))
        assertFalse(ok("content", "@"))
        assertFalse(ok("content", ":80"))
    }

    @Test
    fun ourOwnProvidersAreRejectedWhateverTheyAreCalled() {
        assertFalse(ok("content", own))
        assertFalse(ok("content", "$own.fileprovider"))
        assertFalse(ok("content", "$own.anything.else"))
        assertFalse(ok("content", "COM.MONEYTRACKER.APP"))
        // User info and port do not hide it.
        assertFalse(ok("content", "0@$own.fileprovider"))
        assertFalse(ok("content", "0@$own:8080"))
        // A different package that merely starts with our name is another app.
        assertTrue(ok("content", "com.moneytracker.application"))
        assertTrue(ok("content", "com.moneytracker.app2.provider"))
    }

    @Test
    fun userInfoOfOtherAppsStillWorks() {
        // content://0@media/... is how a cross-user URI is written.
        assertTrue(ok("content", "0@media"))
    }
}
