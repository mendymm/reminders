import java.io.File
import java.nio.file.Files
import java.security.KeyStore
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
                    "Install the NDK version pinned by this project with sdkmanager.",
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

abstract class ValidateReleaseSigning : DefaultTask() {
    @get:Internal
    abstract val keystorePath: Property<String>

    @get:Internal
    abstract val keyAlias: Property<String>

    @get:Internal
    abstract val keystorePassword: Property<String>

    init {
        outputs.upToDateWhen { false }
        notCompatibleWithConfigurationCache(
            "Release signing credentials are supplied through temporary environment variables",
        )
    }

    @TaskAction
    fun validate() {
        val path = keystorePath.orNull?.trim().orEmpty()
        if (path.isEmpty()) {
            throw GradleException("REMINDERS_RELEASE_KEYSTORE is not set")
        }

        val keystoreFile = File(path)
        if (!keystoreFile.isAbsolute) {
            throw GradleException(
                "REMINDERS_RELEASE_KEYSTORE must be an absolute path: $path",
            )
        }
        if (!keystoreFile.isFile) {
            throw GradleException(
                "Release keystore was not found: ${keystoreFile.absolutePath}",
            )
        }
        if (!keystoreFile.canRead()) {
            throw GradleException(
                "Release keystore is not readable: ${keystoreFile.absolutePath}",
            )
        }

        val alias = keyAlias.orNull?.trim().orEmpty()
        if (alias.isEmpty()) {
            throw GradleException("REMINDERS_RELEASE_KEY_ALIAS is not set")
        }

        val password = keystorePassword.orNull.orEmpty()
        if (password.isEmpty()) {
            throw GradleException("REMINDERS_RELEASE_KEYSTORE_PASSWORD is not set")
        }

        val store = try {
            KeyStore.getInstance("PKCS12").apply {
                Files.newInputStream(keystoreFile.toPath()).use { input ->
                    load(input, password.toCharArray())
                }
            }
        } catch (error: Exception) {
            throw GradleException(
                "Unable to open the release PKCS#12 keystore; check its password and contents",
                error,
            )
        }

        if (!store.isKeyEntry(alias)) {
            throw GradleException(
                "Release keystore does not contain a private-key entry named '$alias'",
            )
        }

        try {
            if (store.getKey(alias, password.toCharArray()) == null) {
                throw GradleException(
                    "Release keystore entry '$alias' does not contain a private key",
                )
            }
        } catch (error: GradleException) {
            throw error
        } catch (error: Exception) {
            throw GradleException(
                "Unable to read private-key entry '$alias'; check its password",
                error,
            )
        }
    }
}

val pinnedNdkVersion = "30.0.16248370"
val rustWorkspace = rootProject.layout.projectDirectory.dir("../rust")
val cargoExecutableProvider = providers.gradleProperty("rustCargo").orElse("cargo")
val releaseKeystorePath = providers.environmentVariable("REMINDERS_RELEASE_KEYSTORE")
val releaseKeyAlias = providers.environmentVariable("REMINDERS_RELEASE_KEY_ALIAS")
val releaseKeystorePassword = providers.environmentVariable(
    "REMINDERS_RELEASE_KEYSTORE_PASSWORD",
)
val rustAbis = providers.gradleProperty("rustAbis")
    .map { value ->
        value.split(',')
            .map(String::trim)
            .filter(String::isNotEmpty)
            .also { require(it.isNotEmpty()) { "rustAbis must contain at least one ABI" } }
    }
    .orElse(listOf("arm64-v8a", "x86_64"))

// The Nix build passes the compiled library and the generated Kotlin bindings as
// prebuilt inputs instead of running cargo and uniffi-bindgen from Gradle.
// -PprebuiltJniLibs is a directory laid out as <abi>/libreminder_core.so and
// -PprebuiltUniffiKotlin is the directory of generated Kotlin sources.
val prebuiltJniLibs = providers.gradleProperty("prebuiltJniLibs")
val prebuiltUniffiKotlin = providers.gradleProperty("prebuiltUniffiKotlin")
require(prebuiltJniLibs.isPresent == prebuiltUniffiKotlin.isPresent) {
    "prebuiltJniLibs and prebuiltUniffiKotlin must be set together"
}
val usePrebuiltRust = prebuiltJniLibs.isPresent

val rustInputFiles = files()
if (!usePrebuiltRust) {
    rustInputFiles.from(
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
        minSdk = 37
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += rustAbis.get()
        }
    }

    signingConfigs {
        create("release") {
            storeType = "PKCS12"
            releaseKeystorePath.orNull?.let { path ->
                storeFile = file(path)
            }
            releaseKeyAlias.orNull?.let { alias ->
                keyAlias = alias
            }
            releaseKeystorePassword.orNull?.let { password ->
                storePassword = password
                keyPassword = password
            }
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("release")
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

val validateReleaseSigning = tasks.register<ValidateReleaseSigning>(
    "validateReleaseSigningEnvironment",
) {
    group = "verification"
    description = "Validates the external release-signing keystore and credentials"
    keystorePath.set(releaseKeystorePath.orElse(""))
    keyAlias.set(releaseKeyAlias.orElse(""))
    keystorePassword.set(releaseKeystorePassword.orElse(""))
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

    if (usePrebuiltRust) {
        val jniLibsDirectory = file(prebuiltJniLibs.get())
        val bindingsDirectory = file(prebuiltUniffiKotlin.get())
        require(bindingsDirectory.isDirectory) {
            "prebuiltUniffiKotlin is not a directory: ${bindingsDirectory.absolutePath}"
        }
        rustAbis.get().forEach { abi ->
            val library = File(jniLibsDirectory, "$abi/libreminder_core.so")
            require(library.isFile) {
                "prebuiltJniLibs has no library for ABI $abi: ${library.absolutePath}"
            }
        }
        if (isRelease) {
            tasks.matching { task -> task.name == "pre${capitalizedVariant}Build" }
                .configureEach { dependsOn(validateReleaseSigning) }
        }
        variant.sources.jniLibs?.addStaticSourceDirectory(jniLibsDirectory.absolutePath)
        variant.sources.kotlin?.addStaticSourceDirectory(bindingsDirectory.absolutePath)
        return@onVariants
    }

    val buildRust = tasks.register<CargoNdkBuild>("buildRust$capitalizedVariant") {
        group = "rust"
        description = "Builds reminder-core for the $variantName Android variant"
        rustInputs.from(rustInputFiles)
        workspaceDirectory.set(rustWorkspace)
        ndkDirectory.set(pinnedNdkDirectoryProvider)
        cargoExecutable.set(cargoExecutableProvider)
        abis.set(rustAbis)
        platform.set(37)
        release.set(isRelease)
        outputDirectory.set(layout.buildDirectory.dir("rustJniLibs/$variantName"))
        if (isRelease) {
            dependsOn(validateReleaseSigning)
        }
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
