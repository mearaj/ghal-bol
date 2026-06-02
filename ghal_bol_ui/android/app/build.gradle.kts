import groovy.json.JsonSlurper
import java.io.File
import java.util.Properties

plugins {
    id("com.android.application")
    id("dev.flutter.flutter-gradle-plugin")
}

/** AAR from rustls-platform-verifier-android (coord HTTPS JNI in :p2p). */
fun rustlsPlatformVerifierAar(): File {
    val workspace = rootProject.file("../..")
    val manifest = File(workspace, "ghal_bol/Cargo.toml")
    val out =
        providers.exec {
            workingDir(workspace)
            commandLine(
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--filter-platform",
                "aarch64-linux-android",
                "--manifest-path",
                manifest.absolutePath,
            )
        }.standardOutput.asText.get()
    @Suppress("UNCHECKED_CAST")
    val parsed = JsonSlurper().parseText(out) as Map<String, Any>
    @Suppress("UNCHECKED_CAST")
    val packages = parsed["packages"] as List<Map<String, Any>>
    val entry = packages.first { it["name"] == "rustls-platform-verifier-android" }
    val manifestPath = entry["manifest_path"] as String
    val version = entry["version"] as String
    val crateDir = File(manifestPath).parentFile
    return File(
        crateDir,
        "maven/rustls/rustls-platform-verifier/$version/rustls-platform-verifier-$version.aar",
    )
}

val keystorePropertiesFile = rootProject.file("key.properties")
val keystoreProperties = Properties()
if (keystorePropertiesFile.exists()) {
    keystoreProperties.load(keystorePropertiesFile.inputStream())
}

android {
    namespace = "com.ghalbol"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // Rust JNI: scripts/pack_android_workspace_jni_libs.sh → workspace/build/android-native-ndk/
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs(rootProject.file("../../build/android-native-ndk"))
        }
    }

    defaultConfig {
        applicationId = "com.ghalbol"
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    signingConfigs {
        if (keystorePropertiesFile.exists()) {
            create("release") {
                keyAlias = keystoreProperties["keyAlias"] as String
                keyPassword = keystoreProperties["keyPassword"] as String
                storeFile = file(keystoreProperties["storeFile"] as String)
                storePassword = keystoreProperties["storePassword"] as String
            }
        }
    }

    buildTypes {
        release {
            signingConfig =
                if (keystorePropertiesFile.exists()) {
                    signingConfigs.getByName("release")
                } else {
                    signingConfigs.getByName("debug")
                }
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

dependencies {
    // Kotlin/JNI half of rustls-platform-verifier (coord HTTPS in :p2p).
    implementation(files(rustlsPlatformVerifierAar()))
}
