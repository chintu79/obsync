plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

import java.util.Properties

// Optional release signing. If keystore.properties is present it is used;
// otherwise release builds fall back to the debug keystore so builds never break.
val keystorePropsFile = rootProject.file("app/keystore.properties")
fun keystoreProp(name: String): String? {
    if (!keystorePropsFile.exists()) return null
    val props = Properties()
    keystorePropsFile.inputStream().use { props.load(it) }
    return props.getProperty(name)
}

val rustTargets = mapOf(
    "arm64-v8a"   to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
)

val rustBuildDir = file("${rootProject.projectDir}/../target")
val jniLibsDir = file("${projectDir}/src/main/jniLibs")

tasks.register<Exec>("buildRust") {
    description = "Cross-compile Rust core for Android targets using cargo-ndk"
    workingDir = file("${rootProject.projectDir}/..")
    val targets = rustTargets.values.joinToString(",")
    commandLine("cargo", "ndk", "--target", targets, "--platform", "26", "--", "build", "--release", "-p", "obsync-core")
}

tasks.register<Copy>("copyRustLibs") {
    description = "Copy compiled Rust .so files into jniLibs for APK packaging"
    dependsOn("buildRust")
    rustTargets.forEach { (abi, triple) ->
        from(file("$rustBuildDir/$triple/release/libobsync_core.so")) {
            into("$abi")
        }
    }
    into(jniLibsDir)
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach {
    dependsOn("copyRustLibs")
}

android {
    namespace = "com.obsync"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.obsync"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }

    signingConfigs {
        create("release") {
            val storeFileProp = keystoreProp("storeFile")
            val storePassword = keystoreProp("storePassword")
            val keyAlias = keystoreProp("keyAlias")
            val keyPassword = keystoreProp("keyPassword")
            if (storeFileProp != null && storePassword != null && keyAlias != null && keyPassword != null) {
                storeFile = file(storeFileProp)
                this.storePassword = storePassword
                this.keyAlias = keyAlias
                this.keyPassword = keyPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Sign with our keystore when configured; otherwise fall back to the
            // debug keystore so CI/development release builds never break.
            signingConfig = if (keystorePropsFile.exists()) {
                signingConfigs.getByName("release")
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
    }

    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.ui)
    implementation(libs.androidx.ui.graphics)
    implementation(libs.androidx.ui.tooling.preview)
    implementation(libs.androidx.material3)
    implementation(libs.androidx.material.icons.extended)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.datastore.preferences)
    implementation(libs.camerax.core)
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.android.embedded)
    implementation(libs.kotlinx.coroutines.android)
    debugImplementation(libs.androidx.ui.tooling)
}
