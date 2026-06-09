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

// Play: `flutter build appbundle` → bundleRelease → com.ghalbol
// Dev on device: `flutter run` → com.ghalbol.debug | `flutter run --release` → com.ghalbol
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

    // We bundle the NDK libc++_shared.so (needed by libghal_bol.so / Oboe). If any
    // other dependency ever ships it too, keep the first instead of failing the merge.
    packaging {
        jniLibs {
            pickFirsts += "**/libc++_shared.so"
        }
    }

    defaultConfig {
        applicationId = "com.ghalbol"
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        // Must match scripts/pack_android_workspace_jni_libs.sh (four standard ABIs).
        // Dev fast path: -Pghalbol.arm64Only=true after PACK_ANDROID_ARM64_ONLY=1 pack.
        ndk {
            val abis =
                if (project.findProperty("ghalbol.arm64Only") == "true") {
                    listOf("arm64-v8a")
                } else {
                    listOf("armeabi-v7a", "arm64-v8a", "x86", "x86_64")
                }
            abiFilters.addAll(abis)
        }
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
        debug {
            applicationIdSuffix = ".debug"
        }
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

// Fail fast when Flutter is built without a full native pack (all four ABIs).
val ghalBolJniRoot = rootProject.file("../../build/android-native-ndk")
val ghalBolRequiredAbis =
    listOf("armeabi-v7a", "arm64-v8a", "x86", "x86_64")
tasks.named("preBuild").configure {
    doFirst {
        val arm64Only = project.findProperty("ghalbol.arm64Only") == "true"
        val required = if (arm64Only) listOf("arm64-v8a") else ghalBolRequiredAbis
        val missing =
            required.filter { abi ->
                !File(ghalBolJniRoot, "$abi/libghal_bol.so").isFile ||
                    !File(ghalBolJniRoot, "$abi/libc++_shared.so").isFile
            }
        if (missing.isNotEmpty()) {
            val hint =
                if (arm64Only) {
                    "PACK_ANDROID_ARM64_ONLY=1 ./scripts/pack_android_workspace_jni_libs.sh"
                } else {
                    "./scripts/pack_android_workspace_jni_libs.sh"
                }
            throw GradleException(
                "Missing libghal_bol.so / libc++_shared.so for: ${missing.joinToString()}. " +
                    "From workspace root run: $hint",
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
