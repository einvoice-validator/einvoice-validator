//! Validate EN 16931 and Peppol BIS Billing 3.0 e-invoices against the official
//! validation artefacts, in pure Rust: no JVM, no C library, no FFI.
//!
//! Pre-alpha: the validation engine is not written yet.

/// The version of this crate, as published.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_major_minor_patch() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "{VERSION}");
        for part in parts {
            assert!(
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()),
                "{VERSION}"
            );
        }
    }
}
