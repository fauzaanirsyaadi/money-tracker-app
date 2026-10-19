// A tiny, self-instrumenting test driver for the device scripts in ../../scripts.
//
// It is NOT part of the app and is never shipped: it is a separate APK
// (io.github.fauzaanirsyaadi.moneytracker.uia) whose instrumentation targets itself, so running it does
// not restart or touch the app under test. It exists because real key events sent
// with `adb input text` get dropped when the emulator is busy, whereas the
// accessibility API sets a field's text atomically.
//
//   ./src-tauri/gen/android/gradlew -p tools/uia-driver assembleDebug
//   scripts/uia.mjs installs and calls it.
plugins {
    id("com.android.application") version "9.3.1"
}

android {
    namespace = "io.github.fauzaanirsyaadi.moneytracker.uia"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.fauzaanirsyaadi.moneytracker.uia"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

dependencies {
    // UiAutomator 2: Apache-2.0, Google, maintained (androidx.test). Test tooling only.
    implementation("androidx.test.uiautomator:uiautomator:2.3.0")
    implementation("androidx.test:runner:1.6.2")
    implementation("androidx.test.ext:junit:1.2.1")
    implementation("junit:junit:4.13.2")
}
