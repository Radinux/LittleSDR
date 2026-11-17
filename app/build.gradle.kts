plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

val kitRoot = rootProject.layout.projectDirectory.dir("kit/out").asFile

android {
    namespace = "app.littlesdr"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "app.littlesdr"
        minSdk = 28
        targetSdk = 37
        versionCode = 7
        versionName = "0.4.1"

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
        externalNativeBuild {
            cmake {
                arguments += listOf(
                    "-DANDROID_STL=c++_shared",
                    "-DSDR_KIT_ROOT=${kitRoot.absolutePath}",
                )
                cppFlags += "-std=c++17"
                // Only what the app loads (AGP would otherwise also build libcorrect's CLI tools)
                targets += listOf(
                    "littlesdr", "sdrpp_core", "audio_sink", "radio",
                    "rtl_sdr_source", "airspy_source", "airspyhf_source", "hackrf_source",
                    "rtl_tcp_source", "spyserver_source", "kiwisdr_source", "ft8_decoder",
                )
            }
        }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "4.1.2"
        }
    }

    sourceSets {
        getByName("main") {
            // Prebuilt drivers/DSP libs produced by kit/build-kit.sh
            jniLibs.directories.add(rootProject.file("kit/jniLibs").path)
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Signed with the debug key until a release key is configured
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        jniLibs {
            // Modules are dlopen()ed by name, keep them uncompressed & extracted
            useLegacyPackaging = true
        }
    }
}

// Country database used by the FT8 decoder (callsign -> country), shipped as assets/res/cty
abstract class CtyAssetsTask : DefaultTask() {
    @get:InputDirectory
    abstract val source: DirectoryProperty

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        val out = outputDir.get().asFile.resolve("res/cty")
        out.deleteRecursively()
        out.mkdirs()
        source.get().asFile.listFiles { f -> f.extension == "dat" }?.forEach { it.copyTo(out.resolve(it.name)) }
    }
}

val ctyAssets = tasks.register<CtyAssetsTask>("ctyAssets") {
    source.set(rootProject.layout.projectDirectory.dir("third_party/sdrpp-brown/root/res/cty"))
}

androidComponents {
    onVariants { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(ctyAssets, CtyAssetsTask::outputDir)
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.androidx.datastore)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.kotlinx.serialization.json)
    debugImplementation(libs.compose.ui.tooling)
}
