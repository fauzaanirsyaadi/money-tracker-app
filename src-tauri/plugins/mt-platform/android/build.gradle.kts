import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.plugin.mt_platform"
    compileSdk = 36

    defaultConfig {
        minSdk = 24

        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

dependencies {
    implementation(project(":tauri-android"))
    // Already in the app through appcompat; declared so the plugin compiles on its own.
    implementation("androidx.core:core-ktx:1.13.0")
    // ActivityResult for the file pickers; same version the app already uses.
    implementation("androidx.activity:activity:1.10.1")
    // System fingerprint/face prompt. Apache-2.0, Google, stable 1.1.0 (BiometricPrompt, API 23+).
    implementation("androidx.biometric:biometric:1.1.0")

    // JVM unit tests for the picker URI check. EPL-2.0, test scope only.
    testImplementation("junit:junit:4.13.2")
}
