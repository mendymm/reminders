import java.util.Locale
import javax.inject.Inject
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.FileSystemOperations
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

abstract class CargoNdkBuild @Inject constructor(
    private val execOperations: ExecOperations,
) : DefaultTask() {
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val rustInputs: ConfigurableFileCollection

    @get:Internal
    abstract val workspaceDirectory: DirectoryProperty

    @get:Internal
    abstract val ndkDirectory: DirectoryProperty

    @get:Input
    abstract val cargoExecutable: Property<String>

    @get:Input
    abstract val abis: ListProperty<String>

    @get:Input
    abstract val platform: Property<Int>

    @get:Input
    abstract val release: Property<Boolean>

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @TaskAction
    fun build() {
        val ndk = ndkDirectory.get().asFile
        if (!ndk.isDirectory) {
            throw GradleException(
                "Pinned Android NDK is missing: ${ndk.absolutePath}. " +
                    "Install ndk;28.2.13676358 with sdkmanager.",
            )
        }

        val output = outputDirectory.get().asFile
        output.mkdirs()

        val command = mutableListOf(
            cargoExecutable.get(),
            "ndk",
            "--platform",
            platform.get().toString(),
        )
        abis.get().forEach { abi ->
            command += listOf("--target", abi)
        }
        command += listOf(
            "--output-dir",
            output.absolutePath,
            "build",
            "--package",
            "reminder-core",
        )
        if (release.get()) {
            command += "--release"
        }

        execOperations.exec {
            workingDir(workspaceDirectory.get().asFile)
            environment("ANDROID_NDK_HOME", ndk.absolutePath)
            commandLine(command)
        }.assertNormalExitValue()
    }
}

abstract class GenerateUniFfiBindings @Inject constructor(
    private val execOperations: ExecOperations,
    private val fileSystemOperations: FileSystemOperations,
) : DefaultTask() {
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val jniLibsDirectory: DirectoryProperty

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val rustInputs: ConfigurableFileCollection

    @get:Internal
    abstract val workspaceDirectory: DirectoryProperty

    @get:Input
    abstract val cargoExecutable: Property<String>

    @get:Input
    abstract val metadataAbi: Property<String>

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @TaskAction
    fun generate() {
        val library = jniLibsDirectory.get().file(
            "${metadataAbi.get()}/libreminder_core.so",
        ).asFile
        if (!library.isFile) {
            throw GradleException(
                "Rust library was not produced for binding generation: ${library.absolutePath}",
            )
        }

        val output = outputDirectory.get().asFile
        fileSystemOperations.delete { delete(output) }
        output.mkdirs()

        execOperations.exec {
            workingDir(workspaceDirectory.get().asFile)
            commandLine(
                cargoExecutable.get(),
                "run",
                "--package",
                "reminder-core",
                "--bin",
                "uniffi-bindgen",
                "--",
                "generate",
                library.absolutePath,
                "--language",
                "kotlin",
                "--out-dir",
                output.absolutePath,
            )
        }.assertNormalExitValue()
    }
}

val pinnedNdkVersion = "28.2.13676358"
val rustWorkspace = rootProject.layout.projectDirectory.dir("../rust")
val cargoExecutableProvider = providers.gradleProperty("rustCargo").orElse("cargo")
val rustAbis = providers.gradleProperty("rustAbis")
    .map { value ->
        value.split(',')
            .map(String::trim)
            .filter(String::isNotEmpty)
            .also { require(it.isNotEmpty()) { "rustAbis must contain at least one ABI" } }
    }
    .orElse(listOf("arm64-v8a", "x86_64"))

val rustInputFiles = files(
    fileTree(rustWorkspace) {
        include("**/*.rs")
        include("**/Cargo.toml")
        include("**/uniffi.toml")
        exclude("target/**")
    },
)
val cargoLock = rustWorkspace.file("Cargo.lock").asFile
if (cargoLock.exists()) {
    rustInputFiles.from(cargoLock)
}

android {
    namespace = "dev.mendy.reminders"
    compileSdk {
        version = release(37) {
            minorApiLevel = 1
        }
    }
    ndkVersion = pinnedNdkVersion

    defaultConfig {
        applicationId = "dev.mendy.reminders"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += rustAbis.get()
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    buildFeatures {
        compose = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }
}

androidComponents.onVariants { variant ->
    val variantName = variant.name
    val capitalizedVariant = variantName.replaceFirstChar { character ->
        if (character.isLowerCase()) {
            character.titlecase(Locale.ROOT)
        } else {
            character.toString()
        }
    }
    val isRelease = variant.buildType == "release"
    val pinnedNdkDirectoryProvider = androidComponents.sdkComponents.sdkDirectory.map { sdk ->
        sdk.dir("ndk/$pinnedNdkVersion")
    }

    val buildRust = tasks.register<CargoNdkBuild>("buildRust$capitalizedVariant") {
        group = "rust"
        description = "Builds reminder-core for the $variantName Android variant"
        rustInputs.from(rustInputFiles)
        workspaceDirectory.set(rustWorkspace)
        ndkDirectory.set(pinnedNdkDirectoryProvider)
        cargoExecutable.set(cargoExecutableProvider)
        abis.set(rustAbis)
        platform.set(26)
        release.set(isRelease)
        outputDirectory.set(layout.buildDirectory.dir("rustJniLibs/$variantName"))
    }

    val generateBindings = tasks.register<GenerateUniFfiBindings>(
        "generateUniFfiBindings$capitalizedVariant",
    ) {
        group = "rust"
        description = "Generates Kotlin UniFFI bindings for the $variantName variant"
        dependsOn(buildRust)
        jniLibsDirectory.set(buildRust.flatMap(CargoNdkBuild::outputDirectory))
        rustInputs.from(rustInputFiles)
        workspaceDirectory.set(rustWorkspace)
        cargoExecutable.set(cargoExecutableProvider)
        metadataAbi.set(rustAbis.map { abis -> abis.first() })
        outputDirectory.set(layout.buildDirectory.dir("generated/uniffi/$variantName"))
    }

    variant.sources.jniLibs?.addGeneratedSourceDirectory(
        buildRust,
        CargoNdkBuild::outputDirectory,
    )
    variant.sources.kotlin?.addGeneratedSourceDirectory(
        generateBindings,
        GenerateUniFfiBindings::outputDirectory,
    )
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")

    implementation(composeBom)
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.glance:glance-appwidget:1.2.0")
    implementation("net.java.dev.jna:jna:5.19.1@aar")

    debugImplementation("androidx.compose.ui:ui-tooling")
}
