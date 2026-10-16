package com.plugin.mt_keystore

/**
 * Versioned Keystore aliases and the wrapped-key blob that names its alias.
 *
 * Replacing the wrapping key never deletes the old one first: a *new* alias
 * (`..._v2`) is created, the database key is wrapped with it, the files are
 * swapped, and the old alias is deleted only after the new data is verified.
 * The blob therefore starts with one byte: the version of the alias that wrapped it.
 *
 * Blob layout: `version(1) || iv(12) || ciphertext+tag`.
 */
object AliasVersions {
    const val PREFIX = "money_tracker_db_key_wrap_v"
    const val FIRST = 1
    const val MAX = 255

    fun alias(version: Int): String {
        require(version in FIRST..MAX) { "alias version out of range" }
        return "$PREFIX$version"
    }

    /** The version of an alias of ours, or null for any other alias. */
    fun versionOf(alias: String): Int? {
        if (!alias.startsWith(PREFIX)) return null
        val number = alias.removePrefix(PREFIX)
        // Digits only: "01", "+1" and "1x" are not ours.
        if (number.isEmpty() || !number.all { it in '0'..'9' } || number.length > 3) return null
        if (number.length > 1 && number.startsWith('0')) return null
        return number.toInt().takeIf { it in FIRST..MAX }
    }

    /** Our alias versions among `aliases`, ascending. */
    fun versions(aliases: Collection<String>): List<Int> =
        aliases.mapNotNull { versionOf(it) }.distinct().sorted()

    /** The alias `wrap` uses: the newest one, or the first on a fresh install. */
    fun current(existing: Collection<Int>): Int = existing.maxOrNull() ?: FIRST

    /** The version to create when replacing the key. Never reuses a version. */
    fun next(existing: Collection<Int>): Int {
        val next = (existing.maxOrNull() ?: 0) + 1
        check(next <= MAX) { "no alias versions left" }
        return next
    }

    /** Versions older than `keep`: what may be deleted once `keep` is verified. */
    fun stale(existing: Collection<Int>, keep: Int): List<Int> = existing.filter { it < keep }.sorted()

    fun seal(version: Int, body: ByteArray): ByteArray {
        require(version in FIRST..MAX) { "alias version out of range" }
        return byteArrayOf(version.toByte()) + body
    }

    /** Splits a blob into its alias version and the encrypted body, or null if malformed. */
    fun open(blob: ByteArray, minBody: Int): Pair<Int, ByteArray>? {
        if (blob.size <= 1 + minBody) return null
        val version = blob[0].toInt() and 0xff
        if (version !in FIRST..MAX) return null
        return version to blob.copyOfRange(1, blob.size)
    }
}
