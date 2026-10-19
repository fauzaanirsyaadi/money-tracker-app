package com.moneytracker.uia

import android.os.Bundle
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * One action per run, chosen by instrumentation arguments (`-e action ...`):
 *
 *  - `settext`  `-e index N` (default 0) `-e value TEXT` `[-e clazz android.widget.EditText]`
 *               `[-e focused true]`: sets the text of the Nth matching field of the app
 *               under test, atomically. With `focused true` only the field that has focus
 *               matches (the script focuses it through the page first).
 *  - `click`    `-e text TEXT` or `-e desc DESCRIPTION`
 *  - `wait`     `-e text TEXT` `[-e timeout MS]`: waits until the text is on screen
 *  - `texts`    lists the visible texts (to write selectors)
 *
 * The answer is a status line `result=...` (see scripts/uia.mjs). The WebView's
 * accessibility tree is built lazily: the first query asks for it, so every action
 * first touches the tree and then retries for a few seconds.
 */
class UiaDriver {
    private val args: Bundle = InstrumentationRegistry.getArguments()
    private val device: UiDevice = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
    private val target: String = args.getString("pkg") ?: "io.github.fauzaanirsyaadi.moneytracker"

    private fun report(key: String, value: String) {
        val bundle = Bundle()
        bundle.putString(key, value)
        InstrumentationRegistry.getInstrumentation().sendStatus(0, bundle)
    }

    /** Waits for an object matching `selector`, nudging the WebView to build its tree. */
    private fun find(selector: androidx.test.uiautomator.BySelector, index: Int): UiObject2 {
        val deadline = System.currentTimeMillis() + (args.getString("timeout")?.toLong() ?: 8000L)
        while (System.currentTimeMillis() < deadline) {
            device.waitForIdle(300)
            val all = device.findObjects(selector)
            if (all.size > index) return all[index]
            Thread.sleep(300)
        }
        throw AssertionError("nothing matched $selector at index $index")
    }

    @Test
    fun run() {
        when (val action = args.getString("action")) {
            "settext" -> {
                // `value64` (base64) carries text that would not survive the shell.
                val value = args.getString("value64")?.let { String(android.util.Base64.decode(it, android.util.Base64.DEFAULT), Charsets.UTF_8) }
                    ?: args.getString("value") ?: ""
                val clazz = args.getString("clazz") ?: "android.widget.EditText"
                var selector = By.pkg(target).clazz(clazz)
                if (args.getString("focused") == "true") selector = selector.focused(true)
                val field = find(selector, args.getString("index")?.toInt() ?: 0)
                if (args.getString("focused") != "true") field.click() // focus first, as a user would
                field.text = value
                // No read-back assertion here: a field may legitimately reformat what it
                // is given (the amount field groups digits as 25.000). The caller reads the
                // value from the page itself and decides.
                report("result", "ok")
            }
            "click" -> {
                val text = args.getString("text")
                val desc = args.getString("desc")
                val selector = when {
                    text != null -> By.pkg(target).text(text)
                    desc != null -> By.pkg(target).desc(desc)
                    else -> throw AssertionError("click needs text or desc")
                }
                find(selector, 0).click()
                report("result", "ok")
            }
            "wait" -> {
                val text = args.getString("text") ?: throw AssertionError("wait needs text")
                val timeout = args.getString("timeout")?.toLong() ?: 8000L
                val seen = device.wait(Until.hasObject(By.pkg(target).text(text)), timeout)
                assertNotNull(seen)
                assertTrue("'$text' did not appear", seen == true)
                report("result", "ok")
            }
            "texts" -> {
                device.waitForIdle(500)
                val texts = device.findObjects(By.pkg(target)).mapNotNull { it.text ?: it.contentDescription }
                report("result", texts.filter { it.isNotBlank() }.joinToString(" | "))
            }
            else -> throw AssertionError("unknown action: $action")
        }
    }
}
