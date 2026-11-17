uniffi::setup_scaffolding!();

pub mod model;

/// Proves that Kotlin can call through UniFFI into the Android Rust library.
#[uniffi::export]
pub fn ping() -> String {
    "this is new from Rust".to_owned()
}

#[cfg(test)]
mod tests {
    use super::ping;

    #[test]
    fn ping_identifies_the_rust_core() {
        assert_eq!(ping(), "pong from Rust");
    }
}
