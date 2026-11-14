# Nix build plan

Goal: build the debug APK with Nix so that `aarch64-darwin` and `x86_64-linux` machines need only Nix installed. Release builds are out of scope until the debug build works.

## Decisions

- nixpkgs unstable, plus `rust-overlay` and `crane`.
- Systems: `aarch64-darwin` and `x86_64-linux` only.
- Gradle: whatever nixpkgs unstable ships, as long as it is 9.x. Gradle 8 is not acceptable.
- SDK and NDK from `androidenv.composeAndroidPackages`. As of the check against nixpkgs master, `repo.json` contains platform `37.1` and NDK `30.0.16248370`, so no downgrade is needed. Build-tools `37.0.0` is the latest listed.
- ABIs are one shared variable. The Nix build defaults to `arm64-v8a`. The Gradle-driven flow keeps `arm64-v8a,x86_64` through `gradle.properties`.
- Planned outputs: `apk-arm64` (default), `apk-x86_64`, `apk-universal`. Rust is built once per ABI so the variants share cache.
- Release signing stays outside the sandbox and is a later plan.

## Build shape

```text
androidenv SDK + NDK ---> reminder-core-<abi> (.so, one derivation per ABI)
                              |             \
                              v              \
                       uniffi-kotlin          \
                              \                v
                               +-----------> apk-<variant> (offline Gradle)
```

Inside the Nix build, Gradle receives the prebuilt `.so` files and the Kotlin bindings through Gradle properties, and skips `CargoNdkBuild` and `GenerateUniFfiBindings`. When those properties are unset, the existing Gradle-driven flow is unchanged.

## Stages

Every stage ends with a command the user runs (Fish) and a pass/fail check. The agent does not run `nix`, Cargo, or Gradle. New files must be `git add`ed before Nix can see them.

### Stage 0: Prerequisites (user)

- Install Nix with flakes enabled on the Mac and on the Arch machine.

### Stage 1: Flake skeleton and toolchain

- Create `flake.nix` with the nixpkgs unstable, `rust-overlay`, and `crane` inputs.
- `eachSystem` for the two supported systems, and a shared `abis` list.
- `androidenv.composeAndroidPackages` with platform `37.1`, matching build-tools, NDK `30.0.16248370`, and `accept_license = true`.
- Rust toolchain with the Android targets for the chosen ABIs. Edition 2024 needs Rust 1.85 or later.
- Dev shell: SDK, NDK, JDK, Rust toolchain, `cargo-ndk`, `just`.
- Check: `nix flake lock`, then `nix develop`, then confirm the tools resolve.

### Stage 2: Per-ABI Rust derivations

- `reminder-core-arm64` and `reminder-core-x86_64`, built with `cargo-ndk` and `crane` using dependencies vendored from `Cargo.lock`.
- Check: `nix build .#reminder-core-arm64` and confirm `libreminder_core.so` exists.

### Stage 3: UniFFI bindings derivation

- Build the host `uniffi-bindgen` binary and generate Kotlin from the arm64 `.so`, as `GenerateUniFfiBindings` does today.
- Check: `nix build .#uniffi-kotlin` and confirm the Kotlin files exist.

### Stage 4: Gradle changes for the Nix build

- Edit `android/app/build.gradle.kts` to add `-PprebuiltJniLibs=<dir>` and `-PprebuiltUniffiKotlin=<dir>`. When set, they replace the Cargo and bindgen tasks as the jniLibs and Kotlin generated sources.
- Handle the daemon JVM file, the AGP `aapt2` override, and the build-tools version.
- Check: the existing Gradle-driven debug build still works (user runs it).

### Stage 5: Offline APK build

- `apk` derivation using `gradle.fetchDeps` with `deps.json`, generated per system if needed.
- Handle the read-only SDK problem.
- Check: `nix build .#apk-arm64`, confirm the APK in `result`, install it on the phone.

### Stage 6: Remaining outputs

- Add `apk-x86_64` and `apk-universal`, and set `default` to `apk-arm64`.
- Check: each builds, and the `x86_64` Rust derivation is the only new compile when `apk-universal` follows `apk-arm64`.

### Stage 7: Docs

- Update the "Nix build" section of `AGENTS.md` from planned to the real commands and outputs.
- `README.md` is human-owned and stays untouched unless the user asks.

## Known risks

Ordered by how likely they are to cost time. None of these have been verified by running them.

1. **Offline Gradle dependency lockfile (high likelihood, highest cost).** The sandbox has no network, so every Maven artifact AGP, Kotlin, Compose, Glance, and JNA need must be pre-fetched. `gradle.fetchDeps` records every URL and hash in `deps.json` by running Gradle once through a caching proxy. AGP's graph is large and Gradle resolves some things lazily, so the first recording can miss artifacts and `nix build` fails on a missing one. Mitigation: run the update script, build, fix gaps, repeat.
2. **Lockfile differs per OS (medium).** AGP and its tooling fetch platform-specific artifacts (classifiers such as `linux` and `osx`). A `deps.json` recorded on macOS can lack files that Linux needs, and the reverse. Mitigation: one lockfile per system, chosen by `system` in the flake. The lockfiles are recorded on each machine and regenerated for both after dependency bumps.
3. **AGP wants a writable SDK (medium).** The Nix store is read-only. AGP sometimes writes into the SDK directory (licenses, component checks). Mitigation: copy the SDK out of the store into the build directory before Gradle runs and point `ANDROID_SDK_ROOT` at the copy.
4. **`aapt2` (medium).** AGP downloads `aapt2` from Maven by default. On Arch and macOS the binary runs, so the real problem is the sandbox: the Maven fetch must be in the lockfile. Mitigation: `-Pandroid.aapt2FromMavenOverride=<sdk>/build-tools/<ver>/aapt2`, so the Maven copy is never fetched.
5. **Build-tools version mismatch (medium).** AGP 9.5.1 expects a default build-tools version. If the SDK derivation installs a different one, AGP tries to download it and the sandbox fails. Mitigation: install exactly the version AGP expects, or pin `buildToolsVersion` in `android {}`. nixpkgs lists `37.0.0` as latest; whether AGP 9.5.1 expects it is unchecked.
6. **NDK still required in the Gradle step (medium).** `ndkVersion` is set in `android {}`. Even with prebuilt `.so` files, AGP uses the NDK's strip tool during packaging, including for debug builds. The NDK must be in the SDK given to the Gradle derivation, not only the Rust derivations.
7. **Darwin NDK binaries (medium, Mac only).** The `macosx` NDK 30 archive is marked `arch: all`. Whether its binaries are universal or x86_64-only is unverified. If x86_64-only, Apple Silicon needs Rosetta (`softwareupdate --install-rosetta`, run by the user).
8. **Daemon JVM and JDK (low to medium).** `android/gradle/gradle-daemon-jvm.properties` requests JDK 25 from foojay, which fails offline. Mitigation: remove the file in `postPatch` and use a nixpkgs JDK through `JAVA_HOME`. Check the Gradle 9.x and AGP 9.5.1 compatibility matrix before choosing the JDK, and fall back to 21 if needed. `sourceCompatibility` of 17 is unaffected.
9. **Cross-compiling `rusqlite` with `crane` (low to medium).** `bundled` SQLite compiles C through the `cc` crate and needs the NDK's clang. Inside a derivation, `PATH`, `CC_*`, and sysroot must be right. Mitigation: run `cargo ndk` inside the derivation with `ANDROID_NDK_HOME` set, as the Gradle task does today.
10. **UniFFI bindgen host build (low to medium).** Generating bindings from the Android `.so` already works in the Gradle flow. The host `uniffi-bindgen` build also compiles `reminder-core` and bundled SQLite for the build machine, which needs a working host C compiler in the derivation.
11. **Gradle configuration cache and parallelism (low).** `gradle.properties` enables the configuration cache and parallel execution. Mitigation: pass `--no-configuration-cache` in the Nix build, as `just build-release` already does.
12. **Flake mechanics (low).** Flakes only see git-tracked files, and the whole tracked tree is copied to the store.

The main source of the 2–3 day estimate is risks 1 and 2. Risks 3, 5, and 6 are one-time fixes once found.

## Out of scope

- Release builds and signing.
- Reworking the Gradle-driven flow beyond the prebuilt-input switch.

## Open questions

- Exact nixpkgs Gradle 9.x version and the JDK it pairs with.
- Which build-tools version AGP 9.5.1 expects.
- Whether the Darwin NDK binaries run natively on Apple Silicon.
