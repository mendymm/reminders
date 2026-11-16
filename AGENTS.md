# Agent guidance

## Working rules

- `README.md` is human-owned. Do not add to or edit it unless the user explicitly asks.
- Never run Cargo commands.
- Do not run Gradle builds or tool installation commands; give those commands to the user to run.
- Do not run `nix` commands that build or realize things (`nix build`, `nix develop`, `nix run`, `nix flake lock`, and so on); give them to the user to run. The user has authorized only these quick, non-building checks: `nix eval ...drvPath`, `nix build ... --dry-run`, `nix flake check --no-build`, and running `nixfmt`, `deadnix` and `statix` on `flake.nix` through `nix shell --inputs-from . nixpkgs#...`.
- Ask before running any command that changes files, dependencies, installed tools, or other state unless the user has explicitly authorized that specific change.
- The user uses Fish shell. Commands intended for the user must use Fish syntax.
- Use `apply_patch` for authorized file edits.
- Preserve unrelated user changes in the working tree.

## Product scope

Reminders is a personal Android reminder app for one user. It will be installed directly as an APK, so compatibility decisions do not need to optimize for a broad public audience. Current stable versions are preferred unless a component must remain pinned for compatibility.

## Project layout

```text
reminders/
├── rust/            Cargo workspace and native core library
├── android/         Android Studio project (Kotlin, Compose, and Glance)
├── flake.nix        Nix build: toolchains, Rust, UniFFI bindings, APKs, dev shell
├── flake.lock
├── nix/             Gradle dependency lockfiles, one per system (deps-<system>.json)
├── scripts/         Release signing and release version check (Python)
├── .github/         Release and lockfile-verification workflows, actions.lock
└── Justfile
```

The Rust core is exposed to Kotlin through UniFFI. There are two ways to build the APK:

```text
Gradle-driven (default):  Gradle -> cargo-ndk -> libreminder_core.so -> UniFFI bindgen -> Kotlin -> Compose
Nix (see below):          reminder-core-<abi> (.so) + uniffi-kotlin  ->  offline Gradle  ->  APK
```

`rust/` is a workspace with two crates: `core` (`reminder-core`, the library exposed through UniFFI) and `uniffi-bindgen` (a small binary crate that depends on `uniffi` with the `cli` feature). Keep `uniffi/cli` out of `core` so the Android library does not compile the bindgen toolchain. Both crates share the UniFFI version through `[workspace.dependencies]`.

The initial `ping()` milestone has passed end to end through both build paths, including Rust compilation, UniFFI binding generation, Kotlin compilation, and APK assembly.

## Current toolchain

- Android Gradle Plugin 9.5.1
- Gradle 9.9.1
- Kotlin and Compose compiler plugin 2.4.10
- Android SDK 37.1
- Android NDK 30.0.16248370
- UniFFI 0.32.2
- rusqlite 0.42.0 with bundled SQLite

The Gradle-driven build defaults to `arm64-v8a` and `x86_64`. A phone-only build can use `-PrustAbis=arm64-v8a`. It needs a JDK and the Android SDK/NDK; on the Mac these come from `nix develop` (the machine has no JDK or SDK of its own, and the dev shell includes ktlint so UniFFI formats the generated Kotlin).

The Gradle-driven debug APK is written to `android/app/build/outputs/apk/debug/`.

## Nix build

macOS (`aarch64-darwin`) and Linux (`x86_64-linux`) machines need only Nix installed. The user runs all builds.

Common commands, for the user (Fish, repo root; add `--option substituters https://cache.nixos.org/` if the Determinate cache times out):

```fish
nix develop                                              # JDK 25, Android SDK/NDK, Rust, cargo-ndk, just, ktlint, nixfmt, deadnix, statix, nom
nix build .#apk-arm64                                    # debug APK: result/reminders-debug.apk
nix build .#apk-release-arm64 -o result-release          # unsigned release: result-release/reminders-release-UNSIGNED.zip
python3 scripts/sign-release.py result-release/reminders-release-UNSIGNED.zip reminders-release.apk   # needs `apksigner`; reads the key from 1Password via `op` unless REMINDERS_RELEASE_KEYSTORE is set (e.g. in nix develop)
nix run .#sync-gradle-deps                               # re-record this system's Gradle lockfile (run from the repo root; commit nix/deps-<system>.json)
```

Flake outputs: `reminder-core-<abi>`, `uniffi-kotlin`, and `apk-<variant>` (debug) and `apk-release-<variant>` (unsigned release) for `arm64`, `x86_64`, and `universal`. Rust is built once per ABI so the variants share cache. `apk-arm64`, `apk-release-arm64` and `apk-release-universal` have been built; `apk-x86_64`, `apk-universal` and `apk-release-x86_64` evaluate but have not been built. Each Gradle derivation takes about a minute (configuration, AAR transforms, Kotlin, resources and lint each take a large share).

Decisions and invariants:

- nixpkgs unstable with `rust-overlay` and `crane`. Only `aarch64-darwin` and `x86_64-linux` are supported.
- SDK and NDK come from `androidenv.composeAndroidPackages`: platform 37.1, NDK 30.0.16248370, and build-tools 36.0.0 (AGP 9.5's default and minimum). The Rust derivations use a separate NDK-only composition so SDK changes do not rebuild the Rust crates.
- Use `pkgs.gradle_9` (9.8.2 at the locked nixpkgs revision; AGP 9.5 needs 9.6.0 or later) with `jdk25`. The default `pkgs.gradle` is Gradle 8 and must not be used.
- Inside the Nix build, Gradle receives the prebuilt `.so` files and bindings through `-PprebuiltJniLibs=<dir containing <abi>/libreminder_core.so>` and `-PprebuiltUniffiKotlin=<dir of generated Kotlin>` (both required together) instead of running `cargo ndk` and `uniffi-bindgen` itself. The Gradle-driven flow stays the default when neither is set. `-PrustAbis` must list exactly the ABIs present in `prebuiltJniLibs`.
- Every Nix Gradle derivation also passes `--info --stacktrace --no-configuration-cache`, `-PunsignedRelease=true`, and `-Pandroid.aapt2FromMavenOverride` (the SDK's `build-tools/36.0.0/aapt2`). `gradle-daemon-jvm.properties` is removed in `postPatch` so Gradle does not try to download a JDK.
- Gradle dependencies are served offline from `nix/deps-<system>.json` through `gradle.fetchDeps` (mitm-cache replay, which is why `__darwinAllowLocalNetworking` is set). The lockfiles are generated, never hand-edited, and are recorded with an update task that runs both `assembleDebug` and `assembleRelease`. Re-record on each system after any Gradle dependency change, including `buildscript` classpath dependencies such as tomlj. `nix run .#sync-gradle-deps` does this: it is the `apk-arm64` variant's `mitmCache.updateScript` (all variants share one lockfile and update task; arm64 builds only one Rust ABI first). The script writes `nix/deps-<system>.json` relative to the current directory, so run it from the repository root. The CI runner uses the `x86_64-linux` lockfile, so a dependency change must be recorded there too or the release build fails offline.
- Debug APKs are signed with the committed `android/app/debug.keystore` (standard debug credentials) so they are reproducible and update in place. Never reuse that key for release signing.
- Release APKs are built unsigned (`-PunsignedRelease=true`, output `reminders-release-UNSIGNED.zip`, deliberately not `.apk`) and signed outside Nix by `scripts/sign-release.py <unsigned.zip> <signed.apk>`, which needs `apksigner` on `PATH` or under `$ANDROID_HOME/build-tools` (for example `nix develop`). The environment picks the key source. If `REMINDERS_RELEASE_KEYSTORE` is set it is the `.p12` path and `REMINDERS_RELEASE_KEYSTORE_PASSWORD` is its password (the names Gradle uses; CI does this), and 1Password is never contacted. Otherwise the script fetches the `.p12` and its password with `op read` and keeps the keystore in a private temporary directory (on /dev/shm when available) that is overwritten and removed on every exit path. In both modes the password reaches `apksigner` through its environment, the APK is signed to `<output>.partial`, verified, and only then moved into place. The release key is never given to Nix or to Gradle in the sandbox.
- `scripts/sign-release.py` passes every `apksigner sign` option explicitly: `--min-sdk-version 37` (keep in sync with `minSdk`; the `MIN_SDK_VERSION` constant), v1 off, v2 and v3 on, v4 off, verity off, alignment not preserved with 16 KiB library pages, and debuggable APKs refused. Without explicit flags apksigner added v1 files despite `minSdk` 37. The 1Password references are constants at the top of the script; the item title has a space before the slash.
- The release key is an EC key (P-384, ECDSA with SHA-512). ECDSA signatures are randomized, so two signings of the same input differ, even on one machine. The unsigned zip is the reproducible artifact: compare it, not the signed APK. `--deterministic-dsa-signing` only applies to DSA keys.
- The arm64 `.so`, the arm64 debug APK, and the universal unsigned release are byte-identical across macOS and Linux (verified). Four causes of earlier differences are fixed: (1) Cargo hashes the build host's `rustc -vV` into `-C metadata` of host units (build scripts, proc-macros) and target crates inherit it (rust-lang/cargo#8140; cargo#14107 only covers target units), fixed by the `RUSTC_WRAPPER` in `flake.nix`, which rewrites `-C metadata` to a hash of host-independent inputs; (2) embedded vendor and working-directory paths (per-system store hashes, `/build/source` vs a random macOS directory) changed the linker's string-merge order and debug info, fixed by the same wrapper (`--remap-path-prefix` for the vendor directory, `-Zremap-cwd-prefix` with `RUSTC_BOOTSTRAP=1` set only for the compiler process); (3) the NDK's LLD writes host-specific paths into `.comment`, removed with `llvm-objcopy --remove-section=.comment` after linking; (4) AGP generated a random debug keystore per build, fixed by the committed `debug.keystore`. crane's `doRemapPathPrefix` does nothing unless `craneLib.remapPathPrefixHook` is also in `nativeBuildInputs`, so it is not used. Do not remove or change any of these without re-comparing hashes on both systems.
- `uniffi-kotlin` vendors dependencies as real copies (`cp -rL`), because askama 0.16 breaks under crane's symlinked vendor directory.
- The version comes from `[workspace.package] version` in `rust/Cargo.toml`. The flake reads it for derivation versions (`appVersion`). Gradle parses it with tomlj (a `buildscript` classpath dependency in `android/app/build.gradle.kts`) for `versionName` and derives `versionCode = major*10000 + minor*100 + patch`, so the version must be plain `MAJOR.MINOR.PATCH` with minor and patch below 100. `scripts/check-release-version.py` repeats that formula: keep the two in sync. The `mkApk` source is `lib.fileset.toSource` of `./android` plus `./rust/Cargo.toml` with `sourceRoot = "source/android"`, so Gradle can read `../rust/Cargo.toml` inside the sandbox. A version bump renames every derivation and rebuilds everything.
- Release CI is `.github/workflows/release.yml`, triggered by `vX.Y.Z` tag pushes, in three jobs. `build` has no secrets: it runs `scripts/check-release-version.py` (the tag must equal the Cargo version), `nix build .#apk-release-universal`, and uploads the unsigned zip. `sign` runs no Nix and has the `release` environment's secrets `REMINDERS_RELEASE_KEYSTORE_B64` (base64 `.p12`) and `REMINDERS_RELEASE_KEYSTORE_PASSWORD`; it uses the runner's preinstalled `build-tools/36.0.0` (keep in sync with `androidBuildTools` in `flake.nix`), removes the keystore before any further action runs, and checks `versionName` and `versionCode` of the signed APK with `check-release-version.py --apk`. `publish` has `contents: write`, never sees the key, and runs `gh release create` with `reminders-vX.Y.Z.apk`. The `release` environment should only allow `v*` tags. To release: bump the Cargo version, commit, push, then tag and push the tag. There is deliberately no Nix cache in CI. `check-release-version.py` needs Python 3.11+ (`tomllib`).
- Workflow actions are pinned by `.github/workflows/actions.lock`, generated by `gh actions-lock` (never hand-edited). Rerun it after adding a workflow or changing any `uses:`; `verify-lockfile.yml` runs `gh actions-lock --verify` on pull requests, on pushes to `main`, and weekly.
- Flakes only see git-tracked files. Remind the user to `git add` new files before running Nix.

Update this section when outputs, commands, or invariants change.

## Architecture direction

- Rust owns the SQLite database, schema, migrations, reminder models, recurrence, and eventual sync client.
- Kotlin owns Android-specific plumbing: Compose UI, Glance widgets, AlarmManager scheduling, receivers, and services.
- Keep UniFFI calls coarse and perform blocking store calls away from the UI thread.
- Avoid panics and `unwrap()` in exported Rust code; return typed errors for fallible operations.
- Keep the cold alarm-fire path lightweight by putting immediately needed reminder data in PendingIntent extras.

The next planned milestone is the Rust-backed SQLite `Store` with `open`, `upsert`, and `upcoming` operations.
