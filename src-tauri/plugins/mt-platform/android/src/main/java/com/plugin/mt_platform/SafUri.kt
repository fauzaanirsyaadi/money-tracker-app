package com.plugin.mt_platform

/**
 * Which URIs the system file pickers may hand back to us.
 *
 * `ACTION_CREATE_DOCUMENT` and `ACTION_OPEN_DOCUMENT` are implicit intents, so
 * any installed app can answer them if the user picks it. `ContentResolver`
 * also opens `file://` URIs with this app's own identity, so a `file://` result
 * pointing into our private directory would make us truncate or read our own
 * `db.key`, `money.db` or `lock.json`. Only a `content://` URI served by some
 * other app is accepted.
 */
object SafUri {
    fun isAcceptable(scheme: String?, authority: String?, ownPackage: String): Boolean {
        if (scheme == null || !scheme.equals("content", ignoreCase = true)) return false
        val host = hostOf(authority) ?: return false
        val own = ownPackage.lowercase()
        // Never one of our own providers, whatever it is called.
        if (host == own || host.startsWith("$own.")) return false
        return true
    }

    /** `0@com.example.provider:80` -> `com.example.provider`; null when empty or absent. */
    private fun hostOf(authority: String?): String? {
        val host = authority?.substringAfter('@')?.substringBefore(':')?.lowercase()
        return host?.takeIf { it.isNotEmpty() }
    }
}
