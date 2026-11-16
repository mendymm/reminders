# Nix build plan

Goal: build the debug and release APKs with Nix so that `aarch64-darwin` and `x86_64-linux` machines need only Nix installed, with the release key kept outside Nix.

The agent does not run `nix` builds, Cargo, or Gradle (see `AGENTS.md`); every build and check below is run by the user (Fish). New files must be `git add`ed before Nix can see them.

## Decisions

- nixpkgs unstable, plus `rust-overlay` and `crane`. Systems: `aarch64-darwin` and `x86_64-linux` only.
- Gradle: `pkgs.gradle_9`. The default `pkgs.gradle` is Gradle 8 and must not be used. At the locked nixpkgs revision `gradle_9` is 9.8.2 with `jdk25` as its default JDK. AGP 9.5 requires Gradle 9.6.0 or later, so 9.8.2 is compatible. The wrapper's 9.9.1 is not used in the Nix build.
- SDK and NDK from `androidenv.composeAndroidPackages`: platform `37.1`, NDK `30.0.16248370`, build-tools `36.0.0` (AGP 9.5's default and minimum). The Rust derivations use a separate NDK-only SDK composition, so changing the Gradle-side SDK does not rebuild the Rust crates.
- ABIs are one shared variable. The Gradle-driven flow keeps `arm64-v8a,x86_64` through `gradle.properties`; the Nix variants pass exactly the ABIs they contain.
- Outputs: `reminder-core-<abi>`, `uniffi-kotlin`, `apk-<variant>` (debug) and `apk-release-<variant>` (unsigned release), with variants `arm64`, `x86_64`, `universal`. Rust is built once per ABI so the variants share cache.
- Release signing stays outside the sandbox: Nix produces `reminders-release-UNSIGNED.zip` (not `.apk`, so it cannot be installed by mistake) and `scripts/sign-release.py` signs it on the host with the key from 1Password.
- Debug APKs are signed with the committed `android/app/debug.keystore` (standard debug credentials; public by design). Release signing never uses it.
- Gradle dependencies are served offline by nixpkgs' `gradle.fetchDeps` (mitm-cache replay) from per-system lockfiles `nix/deps-<system>.json`. The lockfiles are recorded with `nix build .#apk-arm64.mitmCache.updateScript` and the script it produces, run from the repository root. The update task is `:app:assembleDebug :app:assembleRelease`, and every variant passes `-PunsignedRelease=true`, so one lockfile covers debug and release.
- Every Gradle derivation passes `--info --stacktrace --no-configuration-cache`, the Stage 4 properties, `-PunsignedRelease=true`, and `-Pandroid.aapt2FromMavenOverride` (the SDK's `build-tools/36.0.0/aapt2`). `gradle-daemon-jvm.properties` is removed in `postPatch`.

## Build shape

```text
androidenv SDK + NDK ---> reminder-core-<abi> (.so, one derivation per ABI)
                              |             \
                              v              \
                       uniffi-kotlin          \
                              \                v
                               +-----------> apk-<variant> / apk-release-<variant> (offline Gradle)
                                                        |
                                    release only        v
                       scripts/sign-release.py (host, key from 1Password) -> signed .apk
```

Inside the Nix build, Gradle receives the prebuilt `.so` files and the Kotlin bindings through Gradle properties and skips `CargoNdkBuild` and `GenerateUniFfiBindings`. When those properties are unset, the Gradle-driven flow is unchanged.

## Stages

### Stage 0: Prerequisites (user)

- Nix with flakes on the Mac and on the Arch machine.

### Stage 1: Flake skeleton and toolchain

- `flake.nix` with the nixpkgs unstable, `rust-overlay`, and `crane` inputs, `eachSystem` for the two systems, the SDK/NDK compositions, the Rust toolchain with the Android targets, and a dev shell (SDK, `jdk25`, Rust toolchain without `rust-docs`, `cargo-ndk`, `just`, `ktlint`, `nixfmt`, `deadnix`, `statix`, `nix-output-monitor`).
- Check: `nix flake lock`, `nix develop`, the tools resolve.

### Stage 2: Per-ABI Rust derivations

- `reminder-core-arm64` and `reminder-core-x86_64`, built with `cargo-ndk` and `crane` from dependencies vendored from `Cargo.lock`. Crane's cargo-log install hook is disabled (`doNotPostBuildInstallCargoBinaries`) because cargo-ndk's output is installed manually.
- Byte reproducibility across hosts needs three things (details under "Reproducibility"): a `RUSTC_WRAPPER`, `.comment` removal, and a fixed debug key.
- Check: `nix build .#reminder-core-arm64`, `libreminder_core.so` exists.

### Stage 3: UniFFI bindings derivation

- The host `uniffi-bindgen` crate generates Kotlin from the arm64 `.so`, as `GenerateUniFfiBindings` does; ktlint is on the derivation's `PATH`, so the output is formatted.
- askama 0.16 fails when its sources are reached through crane's symlinked vendor directory, so this derivation vendors with a real copy (`cp -rL`, then the store path in `config.toml` is rewritten).
- Check: `nix build .#uniffi-kotlin`.

### Stage 4: Gradle changes for the Nix build

- `android/app/build.gradle.kts`: `-PprebuiltJniLibs=<dir>` and `-PprebuiltUniffiKotlin=<dir>` (set together or the build fails) replace the Cargo and bindgen tasks with static jniLibs and Kotlin source directories. It checks that the bindings directory exists and that `libreminder_core.so` exists for every ABI in `-PrustAbis`.
- `-PunsignedRelease=true` builds the release variant without a signing config and without the signing validation, so AGP emits `app-release-unsigned.apk`.
- `signingConfigs.debug` uses the committed `debug.keystore`.
- When none of these properties is set, the Gradle-driven flow is unchanged.

### Stage 5: Offline APK build

- `mkApk { buildType, abiNames }` in `flake.nix` (`apk-arm64` was the first instance). The jniLibs input is a `symlinkJoin` of the per-ABI Rust outputs. Read-only SDK handling was not needed: the SDK is used straight from the store.
- Check: `nix build .#apk-arm64`, install on the phone.

### Stage 6: Remaining outputs

- `apk-x86_64` and `apk-universal` (debug), plus `apk-release-<variant>`.
- Check: each builds, the `x86_64` Rust derivation is the only new compile when `apk-universal` follows `apk-arm64`, and `apk-arm64` keeps the SHA-256 it had before the `symlinkJoin` refactor.

### Stage 7: Release build with the real key

- `apk-release-<variant>` installs the unsigned output as `reminders-release-UNSIGNED.zip`.
- `scripts/sign-release.py <unsigned.zip> <signed.apk>` (standard-library Python; run where `op` is signed in and `apksigner` is on `PATH` or under `$ANDROID_HOME/build-tools`, for example inside `nix develop`). It fetches the `.p12` and its password with `op read`, writes the keystore to a private temporary directory (mode 0700, `/dev/shm` when available) that is overwritten and removed on exit, error, Ctrl-C, SIGTERM and SIGHUP, passes the password to `apksigner` through its environment, and runs `apksigner verify --print-certs --min-sdk-version 37` before writing the output. The 1Password references have a space before the slash in the item name; they are constants at the top of the script.
- `apksigner sign` is called with every option explicit: `--min-sdk-version 37` (keep in sync with `minSdk`), v1 off, v2 on, v3 on, v4 off, `--verity-enabled false`, `--alignment-preserved false` with `--lib-page-alignment 16384`, and `--debuggable-apk-permitted false`. Without explicit flags apksigner added v1 signature files (`META-INF/MANIFEST.MF`, `REMINDER.SF`, `REMINDER.EC`) even though `minSdk` is 37; it now produces the same 960 entries as the unsigned zip. The script was exercised end to end with the real `apksigner` against the unsigned universal release using a throwaway P-384 key (not the real key): it signed and verified.
- The release key is an EC key (P-384, ECDSA with SHA-512). ECDSA signatures are randomized, so two signings of the same unsigned zip differ, even on one machine with the same key (checked: two throwaway-key runs gave different hashes; the Mac and Arch signed APKs differed only in the v2 and v3 signature entries, the central directory fields for the changed v1 file, and the v1 files that are no longer produced). The unsigned zip is the reproducible artifact; compare it, not the signed APK. `--deterministic-dsa-signing` applies to DSA keys only, so it does not help here.
- Check: sign, verify, install over a previous release build (in-place update), and compare the signed APK's certificate fingerprint with the keystore's.

### Stage 8: Docs

- `AGENTS.md` "Nix build" section is current. `README.md` is human-owned and stays untouched unless the user asks.

## Reproducibility

The arm64 `.so` and the arm64 debug APK are byte-identical on `aarch64-darwin` and `x86_64-linux`. Causes of the original differences and what fixes each:

1. Cargo hashes the host line of `rustc -vV` into `-C metadata` of host units (build scripts, proc-macros), and target crates inherit it through their dependencies, so symbol hashes and `TypeId`s differ per host (open upstream: rust-lang/cargo#8140; cargo#14107 covers target units only). Fix: a `RUSTC_WRAPPER` rewrites `-C metadata` to a hash of host-independent inputs (crate name, type, edition, target, cfgs, vendor-relative source path).
2. Embedded paths change the linker's string-merge order and debug info: vendored-crate paths contain per-system store hashes, and the build directory is `/build/source` on Linux but random on macOS. Fix: the wrapper also adds `--remap-path-prefix` for the vendor directory and `-Zremap-cwd-prefix` (needs `RUSTC_BOOTSTRAP=1`, set for the compiler process only) for `DW_AT_comp_dir`.
3. The NDK's LLD writes host-specific build paths into `.comment`. Fix: `llvm-objcopy --remove-section=.comment` after linking.
4. AGP generated a random debug keystore per build. Fix: the committed `debug.keystore`.

crane's `doRemapPathPrefix` does nothing unless `craneLib.remapPathPrefixHook` is also in `nativeBuildInputs`, so it is not used. Do not remove or change these without re-comparing hashes on both systems.

## Performance notes

Gradle timing from one `apk-release-arm64` build on `aarch64-darwin` (about 61 s of log, tasks overlap, timestamps arrive in bursts):

| Window | Time | What |
|---|---|---|
| 0 to 18 s | 29% | Daemon fork, settings and plugin resolution, plugin-classpath instrumentation transforms, `build.gradle.kts` compilation and AGP setup |
| 18 to 32 s | 23% | Dependency resolution and AAR transforms (about 63 AARs) |
| 32 to 48 s | 25% | Kotlin compile (Compose compiler), one silent 10 s stretch |
| 48 to 61 s | 21% | AAPT2 resource compilation (about 1,000 XML files), lint-vital, dexing, packaging |

Not applied: skipping the lint-vital tasks for the Nix release (about 3 to 5 s, loses a lint check and shrinks the lockfile), and avoiding the single-use daemon fork caused by `-Xmx2g` in `gradle.properties` (about 1 s). Each variant is a separate derivation and pays roughly this cost again.

## Risks and outcomes

1. **Offline Gradle lockfile (was high).** The recording works for debug and release on `aarch64-darwin` and for debug on `x86_64-linux`. After any dependency change, re-record on each system.
2. **Lockfile differs per OS (medium).** One lockfile per system, chosen by `system`.
3. **Writable SDK (not triggered).** AGP builds against the read-only store SDK in the dev shell and in the sandbox.
4. **`aapt2` (works).** The override avoids the Maven copy; AGP prints a harmless "option is experimental" warning (`android.sync.suppressAgpWarnings=UNSUPPORTED_PROJECT_OPTION_USE` silences it).
5. **Build-tools mismatch (resolved).** 36.0.0 is installed.
6. **NDK needed in the Gradle step (resolved).** AGP strips native libraries with it, so the full SDK composition includes it.
7. **Darwin NDK binaries (resolved).** They run on Apple Silicon; whether natively or through Rosetta is unknown.
8. **Daemon JVM (resolved).** The properties file is removed and `jdk25` is used.
9. **Bundled SQLite (resolved).** `libsqlite3-sys` compiles with the NDK inside the derivation even though `rusqlite` is not used by the code yet.
10. **UniFFI bindgen host build (resolved).** Askama needs the real-copy vendor directory.
11. **Configuration cache (resolved).** `--no-configuration-cache` is passed.
12. **Flake mechanics (standing).** Only tracked files are visible to Nix.

## Out of scope

- Reworking the Gradle-driven flow beyond the prebuilt-input and unsigned-release switches.

## Status

| Item | State |
|---|---|
| Stage 1 flake, toolchains, dev shell | Done |
| Stage 2 `reminder-core-arm64` | Done; byte-identical on Mac and Linux |
| Stage 2 `reminder-core-x86_64` | Built on both systems as part of `apk-release-universal` |
| Stage 3 `uniffi-kotlin` | Done (built on both systems as an APK dependency) |
| Stage 4 Gradle switches | Verified in the dev shell; the Gradle-driven flow still builds |
| Stage 5 `apk-arm64` | Done; byte-identical on Mac and Linux; installed on the phone |
| Stage 6 `apk-release-universal` (unsigned) | Built on both systems; byte-identical (SHA-256 `96f86a3f…ef113`, 27,573,577 bytes, 960 entries, `lib/arm64-v8a/` and `lib/x86_64/` each with `libreminder_core.so`, `libjnidispatch.so` and `libandroidx.graphics.path.so`, no signing block and no v1 signature files) |
| Stage 6 `apk-x86_64`, `apk-universal`, `apk-release-x86_64` | Evaluate; not built |
| Stage 6 `apk-arm64` hash after the `symlinkJoin` refactor | Not compared |
| Stage 7 `apk-release-arm64` (unsigned) | Built on `aarch64-darwin` |
| Stage 7 signing with the real key | Done on both machines with the earlier script version (which still produced v1 files): the signed APKs verified, share the same certificate (EC P-384, SHA-256 prefix `ac17ac5d`), and differ only in the randomized signature parts. Not yet repeated with the explicit-flags script |
| Stage 7 install of a release APK over a previous release build | Not confirmed |
| Linux lockfile covers the release build | Yes: `apk-release-universal` built on `x86_64-linux` |

Next:

1. Run `apk-arm64` again on both systems and compare the hash with the earlier one.
2. Build `apk-x86_64`, `apk-universal` and `apk-release-arm64` on Linux if wanted; the universal release already proves the x86_64 library and the `symlinkJoin` path work.
3. Re-sign `reminders-release-UNSIGNED.zip` with the explicit-flags `scripts/sign-release.py`, check the result has 960 entries and verifies, and install it over a previous release build to confirm in-place updates. Also check that the native libraries are 16 KiB aligned (not checked yet).
