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
    // System fingerprint/face prompt. Apache-2.0, Google, stable 1.1.0 (BiometricPrompt, API 23+).
    implementation("androidx.biometric:biometric:1.1.0")
}
