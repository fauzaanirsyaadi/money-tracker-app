# uia-driver (experiment, not part of the app)

A self-instrumenting test APK (`io.github.fauzaanirsyaadi.moneytracker.uia`) that sets a field's text through
the accessibility API (UiAutomator) instead of `adb input text`. Built and used only by
`scripts/android_smoke.mjs --uia` on an **emulator or test device**.

* Not shipped: it is a separate Gradle project, not referenced by the app's build, the
  release APK checks or CI. Do not install it on a personal phone.
* The default smoke run does not use it: key events with verification and retry were
  enough (they passed under CPU load), and this adds a second APK to maintain.
* Known: UiAutomator only sees the WebView's DOM after a first query builds the lazy
  accessibility tree (the driver retries for that). A field that reformats its text (the
  amount field) is verified by the script from the page, not by the driver.

Build: `./src-tauri/gen/android/gradlew -p tools/uia-driver assembleDebug`
