# Agent guidance

## Working rules

- `README.md` is human-owned. Do not add to or edit it unless the user explicitly asks.
- Never run Cargo commands.
- Do not run Gradle builds or tool installation commands; give those commands to the user to run.
- Do not run `nix` commands (`nix build`, `nix develop`, `nix flake lock`, and so on); give them to the user to run.
- Ask before running any command that changes files, dependencies, installed tools, or other state unless the user has explicitly authorized that specific change.
- The user uses Fish shell. Commands intended for the user must use Fish syntax.
- Use `apply_patch` for authorized file edits.
- Preserve unrelated user changes in the working tree.

## Product scope

Reminders is a personal Android reminder app for one user. It will be installed directly as an APK, so compatibility decisions do not need to optimize for a broad public audience. Current stable versions are preferred unless a component must remain pinned for compatibility.

## Project layout

```text
reminders/
├── rust/       Cargo workspace and native core library
└── android/    Android Studio project (Kotlin, Compose, and Glance)
```

The Rust core is exposed to Kotlin through UniFFI. Gradle owns the Android pipeline:

```text
Gradle -> cargo-ndk -> libreminder_core.so -> UniFFI bindgen -> Kotlin -> Compose
```

`rust/` is a workspace with two crates: `core` (`reminder-core`, the library exposed through UniFFI) and `uniffi-bindgen` (a small binary crate that depends on `uniffi` with the `cli` feature). Keep `uniffi/cli` out of `core` so the Android library does not compile the bindgen toolchain. Both crates share the UniFFI version through `[workspace.dependencies]`.

The initial `ping()` milestone has passed end to end. `./gradlew :app:assembleDebug` completed successfully, including Rust compilation, UniFFI binding generation, Kotlin compilation, and APK assembly. UniFFI emitted a harmless warning because ktlint is not installed; optional generated-code formatting may be configured later.

## Current toolchain

- Android Gradle Plugin 9.5.1
- Gradle 9.9.1
- Kotlin and Compose compiler plugin 2.4.10
- Android SDK 37.1
- Android NDK 30.0.16248370
- UniFFI 0.32.2
- rusqlite 0.42.0 with bundled SQLite

The app builds `arm64-v8a` and `x86_64` by default. A phone-only build can use `-PrustAbis=arm64-v8a`.

The debug APK is written to `android/app/build/outputs/apk/debug/`.

## Nix build (planned, not implemented yet)

The goal is to build the APK with Nix so that macOS (`aarch64-darwin`) and Linux (`x86_64-linux`) machines need only Nix installed. Decisions made so far:

- nixpkgs unstable, with `rust-overlay` and `crane`. Only `aarch64-darwin` and `x86_64-linux` are supported.
- The Android SDK and NDK come from `androidenv.composeAndroidPackages`: platform 37.1 and NDK 30.0.16248370 as listed under Current toolchain, plus build-tools 36.0.0 (AGP 9.5's default and minimum). The Rust derivations use a separate NDK-only composition so SDK changes do not rebuild the Rust crates.
- Use `pkgs.gradle_9` (9.8.2 at the locked nixpkgs revision; AGP 9.5 needs 9.6.0 or later) with `jdk25`. The default `pkgs.gradle` is Gradle 8 and must not be used.
- The pure build is a derivation chain: per-ABI `reminder-core` `.so` files, then UniFFI Kotlin bindings, then an offline Gradle APK build with dependencies locked by `gradle.fetchDeps` (`deps.json`, likely per system).
- Inside the Nix build, Gradle receives the prebuilt `.so` files and bindings through `-PprebuiltJniLibs=<dir containing <abi>/libreminder_core.so>` and `-PprebuiltUniffiKotlin=<dir of generated Kotlin>` (both required together) instead of running `cargo ndk` and `uniffi-bindgen` itself. The existing Gradle-driven flow stays the default when neither property is set. `-PrustAbis` must still list exactly the ABIs present in `prebuiltJniLibs`.
- The Gradle dependency lockfiles are `nix/deps-<system>.json`. They are generated, not hand-edited: run the script from `nix build .#apk-arm64.mitmCache.updateScript` from the repository root after any Gradle dependency change, on each supported system.
- Debug APKs are signed with the committed `android/app/debug.keystore` (standard debug credentials) so they are reproducible and update in place. Never reuse that key for release signing.
- `android/gradle/gradle-daemon-jvm.properties` is removed inside the Nix build so Gradle does not try to download a JDK.
- ABIs are one parameter shared by the Rust derivations, toolchain targets, and `-PrustAbis`. Flake outputs: `reminder-core-<abi>`, `uniffi-kotlin`, and `apk-<variant>` (debug) and `apk-release-<variant>` (unsigned release) for `arm64`, `x86_64`, and `universal`. One Rust derivation per ABI so they share cache.
- Release APKs are built unsigned by Nix (`-PunsignedRelease=true`, output `reminders-release-UNSIGNED.zip`) and signed on the host with `scripts/sign-release.py <unsigned.zip> <signed.apk>`, which reads the key and password from 1Password with `op read`. The release key lives outside the repo and is never given to Nix or to Gradle in the sandbox.
- Flakes only see git-tracked files. Remind the user to `git add` new files before running Nix.

Update this section as stages land, and replace "planned" with the actual outputs and commands.

## Architecture direction

- Rust owns the SQLite database, schema, migrations, reminder models, recurrence, and eventual sync client.
- Kotlin owns Android-specific plumbing: Compose UI, Glance widgets, AlarmManager scheduling, receivers, and services.
- Keep UniFFI calls coarse and perform blocking store calls away from the UI thread.
- Avoid panics and `unwrap()` in exported Rust code; return typed errors for fallible operations.
- Keep the cold alarm-fire path lightweight by putting immediately needed reminder data in PendingIntent extras.

The next planned milestone is the Rust-backed SQLite `Store` with `open`, `upsert`, and `upcoming` operations.
