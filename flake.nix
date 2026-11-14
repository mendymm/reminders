{
  description = "Reminders: Android reminder app with a Rust core";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      crane,
      ...
    }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];

      forEachSystem = lib.genAttrs systems;

      # Android pins. Keep in sync with android/app/build.gradle.kts.
      androidPlatform = 37;
      androidPlatformVersion = "37.1";
      androidBuildTools = "36.0.0";
      androidNdk = "30.0.16248370";

      # Short name -> Android ABI and Rust target. One Rust derivation is built
      # per entry so APK variants share the compiled libraries.
      androidAbis = {
        arm64 = {
          abi = "arm64-v8a";
          target = "aarch64-linux-android";
        };
        x86_64 = {
          abi = "x86_64";
          target = "x86_64-linux-android";
        };
      };
      rustTargets = lib.mapAttrsToList (_: entry: entry.target) androidAbis;

      perSystem =
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };

          # Full SDK for Gradle and the dev shell. AGP 9.5 defaults to build-tools
          # 36.0.0 and uses the NDK to strip native libraries during packaging.
          androidComposition = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ androidPlatformVersion ];
            buildToolsVersions = [ androidBuildTools ];
            includeNDK = true;
            ndkVersions = [ androidNdk ];
          };
          androidSdk = androidComposition.androidsdk;
          androidSdkRoot = "${androidSdk}/libexec/android-sdk";
          androidNdkHome = "${androidSdkRoot}/ndk/${androidNdk}";

          # NDK-only SDK for the Rust derivations, so changing the Gradle-side SDK
          # components does not rebuild the Rust crates.
          rustNdkComposition = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ ];
            buildToolsVersions = [ ];
            includeCmake = false;
            includeNDK = true;
            ndkVersions = [ androidNdk ];
          };
          rustNdkHome = "${rustNdkComposition.androidsdk}/libexec/android-sdk/ndk/${androidNdk}";

          buildToolchain = pkgs.rust-bin.stable.latest.minimal.override {
            targets = rustTargets;
          };
          # The default profile also ships rust-docs, which is slow to unpack and unused.
          devToolchain = pkgs.rust-bin.stable.latest.minimal.override {
            targets = rustTargets;
            extensions = [
              "clippy"
              "rust-src"
              "rustfmt"
            ];
          };

          craneLib = (crane.mkLib pkgs).overrideToolchain buildToolchain;

          # The Cargo workspace. uniffi.toml is kept because bindgen reads it.
          rustSrc = lib.fileset.toSource {
            root = ./rust;
            fileset = lib.fileset.unions [
              (craneLib.fileset.commonCargoSources ./rust)
              ./rust/core/uniffi.toml
            ];
          };

          # Cargo hashes the build host's `rustc -vV` (including the host triple) into
          # the `-C metadata` of host units (build scripts and proc-macros). Target
          # crates inherit that through their dependencies, so the same source gets
          # different symbol hashes on different build hosts (rust-lang/cargo#8140).
          # Replace the value with a hash of inputs that do not depend on the host.
          #
          # The wrapper also makes the paths that rustc embeds host independent, since
          # the linker's string merging order depends on them. Vendored crate paths and
          # their working directories contain store hashes that differ per build host,
          # so they are remapped to the placeholder hash crane substitutes afterwards.
          # `-Zremap-cwd-prefix` (the only way to remap DW_AT_comp_dir) needs
          # RUSTC_BOOTSTRAP, which is set for the compiler process only.
          rustcWrapper = pkgs.writeShellScript "rustc-stable-metadata" ''
            set -eu
            rustc=$1
            shift
            args=("$@")
            key=""
            meta=""
            vendor=""
            i=0
            while [ "$i" -lt "''${#args[@]}" ]; do
              arg=''${args[$i]}
              case "$arg" in
                metadata=*)
                  meta=$i
                  ;;
                --crate-name | --crate-type | --edition | --target | --cfg)
                  i=$((i + 1))
                  key="$key $arg ''${args[$i]}"
                  ;;
                --crate-name=* | --crate-type=* | --edition=* | --target=*)
                  key="$key $arg"
                  ;;
                *.rs)
                  case "$arg" in
                    *-vendor-cargo-deps/*)
                      vendor=''${arg%%-vendor-cargo-deps/*}-vendor-cargo-deps
                      arg=''${arg#*-vendor-cargo-deps/}
                      arg=''${arg#*/}
                      ;;
                  esac
                  key="$key $arg"
                  ;;
              esac
              i=$((i + 1))
            done
            if [ -z "$meta" ]; then
              exec "$rustc" "''${args[@]}"
            fi
            hash=$(printf '%s' "$key" | ${pkgs.coreutils}/bin/sha256sum | ${pkgs.coreutils}/bin/cut -c1-16)
            args[$meta]="metadata=$hash"
            placeholder=/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
            if [ -n "$vendor" ]; then
              args+=("--remap-path-prefix=$vendor=$placeholder-vendor-cargo-deps")
            fi
            cwd=$(pwd -P)
            case "$cwd" in
              /nix/store/*-cargo-package-*)
                package=''${cwd##*/}
                args+=("-Zremap-cwd-prefix=$placeholder-''${package#*-}")
                ;;
              *)
                args+=("-Zremap-cwd-prefix=/build/source")
                ;;
            esac
            RUSTC_BOOTSTRAP=1 exec "$rustc" "''${args[@]}"
          '';

          mkReminderCore =
            { abi, ... }:
            let
              buildCommand = ''
                cargo ndk \
                  --platform ${toString androidPlatform} \
                  --target ${abi} \
                  --output-dir "$PWD/jniLibs" \
                  build --package reminder-core --release
              '';
              commonArgs = {
                src = rustSrc;
                pname = "reminder-core-${abi}";
                version = "0.1.0";
                strictDeps = true;
                doCheck = false;
                nativeBuildInputs = [ pkgs.cargo-ndk ];
                ANDROID_NDK_HOME = rustNdkHome;
                buildPhaseCargoCommand = buildCommand;
                RUSTC_WRAPPER = rustcWrapper;
              };
              cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            in
            craneLib.buildPackage (
              commonArgs
              // {
                inherit cargoArtifacts;
                # Binaries are installed by installPhaseCommand from cargo-ndk's output,
                # not from a cargo build log.
                doNotPostBuildInstallCargoBinaries = true;
                installPhaseCommand = ''
                  # The linker banner in .comment embeds the NDK's build-host paths.
                  for lib in jniLibs/*/*.so; do
                    "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-objcopy \
                      --remove-section=.comment "$lib"
                  done
                  mkdir -p "$out"
                  cp -r jniLibs/. "$out/"
                '';
              }
            );

          reminderCore = lib.mapAttrs (_: mkReminderCore) androidAbis;

          # crane vendors crates as symlinks into the store. askama (used by
          # uniffi_bindgen) builds include paths relative to the symlink but
          # canonicalizes the target, which breaks. Vendor with real copies instead.
          vendorDir = craneLib.vendorCargoDeps { src = rustSrc; };
          vendorCopy = pkgs.runCommand "vendor-cargo-deps-copy" { } ''
            cp -rL ${vendorDir} "$out"
            chmod -R u+w "$out"
            substituteInPlace "$out/config.toml" --replace-fail "${vendorDir}" "$out"
          '';

          # Host build of the bindgen crate, which only depends on `uniffi` with `cli`.
          uniffiBindgen = craneLib.buildPackage {
            src = rustSrc;
            pname = "uniffi-bindgen";
            version = "0.1.0";
            strictDeps = true;
            doCheck = false;
            cargoVendorDir = vendorCopy;
            cargoExtraArgs = "--locked --package uniffi-bindgen";
          };

          # Kotlin bindings, generated from the arm64 library the same way the Gradle
          # GenerateUniFfiBindings task does. Library mode reads the crate's
          # uniffi.toml through `cargo metadata`, so it needs cargo, the workspace
          # sources, and the vendored dependencies.
          uniffiKotlin =
            pkgs.runCommand "reminder-core-uniffi-kotlin"
              {
                nativeBuildInputs = [
                  buildToolchain
                  uniffiBindgen
                  pkgs.ktlint
                ];
              }
              ''
                export HOME="$TMPDIR"
                export CARGO_HOME="$TMPDIR/cargo-home"
                mkdir -p "$CARGO_HOME" work
                cp ${vendorCopy}/config.toml "$CARGO_HOME/config.toml"
                cp -r ${rustSrc}/. work
                chmod -R u+w work
                cd work
                CARGO_NET_OFFLINE=true uniffi-bindgen generate \
                  ${reminderCore.arm64}/arm64-v8a/libreminder_core.so \
                  --language kotlin \
                  --out-dir "$out"
              '';
        in
        {
          packages =
            lib.mapAttrs' (name: drv: lib.nameValuePair "reminder-core-${name}" drv) reminderCore
            // {
              uniffi-kotlin = uniffiKotlin;
            };

          devShells.default = pkgs.mkShell {
            packages = [
              androidSdk
              pkgs.jdk25
              devToolchain
              pkgs.cargo-ndk
              pkgs.just
              pkgs.nixfmt
              pkgs.deadnix
              pkgs.statix
              pkgs.nix-output-monitor
              pkgs.ktlint
            ];

            ANDROID_HOME = androidSdkRoot;
            ANDROID_SDK_ROOT = androidSdkRoot;
            ANDROID_NDK_HOME = androidNdkHome;
          };
        };

      outputs = forEachSystem perSystem;
      pick = name: lib.mapAttrs (_: perSystemOutputs: perSystemOutputs.${name}) outputs;
    in
    {
      packages = pick "packages";
      devShells = pick "devShells";
    };
}
