import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "app.littlesdr"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "app.littlesdr"
        minSdk = 28
        targetSdk = 37
        versionCode = 25
        versionName = "1.5.2"

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    // Release key: kept OUTSIDE the repository. Its location and passwords come from
    // ~/.gradle/gradle.properties (LITTLESDR_STORE_FILE, LITTLESDR_STORE_PASSWORD,
    // LITTLESDR_KEY_ALIAS, LITTLESDR_KEY_PASSWORD). Without them (anyone building the public
    // repo), releases are signed with the debug key so the build still works.
    val releaseStore = providers.gradleProperty("LITTLESDR_STORE_FILE").orNull?.let { file(it) }?.takeIf { it.exists() }
    signingConfigs {
        if (releaseStore != null) create("release") {
            storeFile = releaseStore
            storePassword = providers.gradleProperty("LITTLESDR_STORE_PASSWORD").get()
            keyAlias = providers.gradleProperty("LITTLESDR_KEY_ALIAS").get()
            keyPassword = providers.gradleProperty("LITTLESDR_KEY_PASSWORD").get()
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

}

// Native layer, all Rust: the engine (engine/) built by cargo-ndk into liblittlesdr.so, next to
// the prebuilt receiver drivers (kit/jniLibs, from kit/build-kit.sh): librtlsdr + libusb, linked;
// libairspy + libairspyhf, loaded when an Airspy is opened
abstract class RustJniLibsTask : DefaultTask() {
    @get:Inject
    abstract val exec: ExecOperations

    @get:Internal
    abstract val engineDir: DirectoryProperty

    @get:Internal
    abstract val kitLibs: DirectoryProperty

    @get:Input
    abstract val ndkDir: Property<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    init {
        // cargo knows what is up to date
        outputs.upToDateWhen { false }
    }

    @TaskAction
    fun build() {
        val out = outputDir.get().asFile
        val abis = listOf("arm64-v8a", "x86_64")
        val cargo = File(System.getProperty("user.home"), ".cargo/bin/cargo").takeIf { it.exists() }?.path ?: "cargo"
        exec.exec {
            workingDir = engineDir.get().asFile
            environment("ANDROID_NDK_HOME", ndkDir.get())
            commandLine(listOf(cargo, "ndk") + abis.flatMap { listOf("-t", it) } + listOf("-P", "28", "-o", out.path, "build", "--release", "--lib", "--no-default-features", "--features", "rtlsdr"))
        }
        for (abi in abis) {
            // libairspy / libairspyhf are loaded by the engine when an Airspy is opened
            // libfaad_drm: FAAD2's DRM build (DRM AAC stations), loaded by the engine too
            for (lib in listOf("librtlsdr.so", "libusb1.0.so", "libairspy.so", "libairspyhf.so", "libfaad_drm.so")) {
                kitLibs.get().asFile.resolve("$abi/$lib").copyTo(out.resolve("$abi/$lib"), overwrite = true)
            }
        }
    }
}

val rustJniLibs = tasks.register<RustJniLibsTask>("rustJniLibs") {
    engineDir.set(rootProject.layout.projectDirectory.dir("engine"))
    kitLibs.set(rootProject.layout.projectDirectory.dir("kit/jniLibs"))
    ndkDir.set(androidComponents.sdkComponents.sdkDirectory.map { it.dir("ndk/${android.ndkVersion}").asFile.path })
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
        variant.sources.jniLibs?.addGeneratedSourceDirectory(rustJniLibs, RustJniLibsTask::outputDir)
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
