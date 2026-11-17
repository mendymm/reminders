uniffi::setup_scaffolding!();

pub mod instant;
pub mod model;
pub mod progress;
pub mod recurrence;

/// Proves that Kotlin can call through UniFFI into the Android Rust library.
#[uniffi::export]
pub fn ping() -> String {
    "this is new from Rust".to_owned()
}
