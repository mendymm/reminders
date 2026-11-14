# Agent guidance

## Working rules

- `README.md` is human-owned. Do not add to or edit it unless the user explicitly asks.
- Never run Cargo commands.
- Do not run Gradle builds or tool installation commands; give those commands to the user to run.
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

The initial `ping()` milestone has passed end to end. `./gradlew :app:assembleDebug` completed successfully, including Rust compilation, UniFFI binding generation, Kotlin compilation, and APK assembly. UniFFI emitted a harmless warning because ktlint is not installed; optional generated-code formatting may be configured later.

## Current toolchain

- Android Gradle Plugin 9.5.1
- Gradle 9.9.1
- Kotlin and Compose compiler plugin 2.4.10
- Android SDK 37.1
- Android NDK 28.2.13676358
- UniFFI 0.32.2
- rusqlite 0.42.0 with bundled SQLite

The app builds `arm64-v8a` and `x86_64` by default. A phone-only build can use `-PrustAbis=arm64-v8a`.

The debug APK is written to `android/app/build/outputs/apk/debug/`.

## Architecture direction

- Rust owns the SQLite database, schema, migrations, reminder models, recurrence, and eventual sync client.
- Kotlin owns Android-specific plumbing: Compose UI, Glance widgets, AlarmManager scheduling, receivers, and services.
- Keep UniFFI calls coarse and perform blocking store calls away from the UI thread.
- Avoid panics and `unwrap()` in exported Rust code; return typed errors for fallible operations.
- Keep the cold alarm-fire path lightweight by putting immediately needed reminder data in PendingIntent extras.

The next planned milestone is the Rust-backed SQLite `Store` with `open`, `upsert`, and `upcoming` operations.
