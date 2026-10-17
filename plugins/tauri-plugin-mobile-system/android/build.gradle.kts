plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.okhsunrog.mobile_system"
    compileSdk = 37

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    implementation("com.onyx.android.sdk:onyxsdk-pen:1.5.8")
    implementation("org.lsposed.hiddenapibypass:hiddenapibypass:6.2")
    implementation("androidx.core:core-ktx:1.19.1")
    implementation(project(":tauri-android"))
}
