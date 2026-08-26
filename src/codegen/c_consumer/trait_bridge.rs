//! C symbols the FFI backend exports for a `[[trait_bridges]]` entry.
//!
//! The register symbol is **not** derived from the trait name: `backends::ffi::trait_bridge`
//! gates the whole registration block on `TraitBridgeConfig::register_fn` and appends that
//! configured function name to the prefix verbatim. A consumer that spells
//! `{prefix}_register_{trait_snake}` instead links against nothing whenever `register_fn` names
//! anything else. The unregister/clear symbols *are* trait-derived, which is why they take the
//! trait name and this module — not the caller — applies the casing. ~keep

use crate::codegen::generators::trait_bridge::trait_snake_of;

/// The exported C symbol for a trait bridge's `register` entry point:
/// `{prefix}_{register_fn}`.
///
/// `register_fn` is the configured name from `[[crates.trait_bridges]]`, used verbatim. It is
/// deliberately not re-cased: it is already a snake-case Rust function name, and re-casing would
/// silently rewrite a symbol consumers link against. ~keep
pub fn trait_register_symbol(prefix: &str, register_fn: &str) -> String {
    format!("{prefix}_{register_fn}")
}

/// The exported C symbol for a trait bridge's `unregister` entry point:
/// `{prefix}_unregister_{trait_snake}`. Derived from the trait name, not from the configured
/// `unregister_fn`. ~keep
pub fn trait_unregister_symbol(prefix: &str, trait_name: &str) -> String {
    format!("{prefix}_unregister_{}", trait_snake_of(trait_name))
}

/// The exported C symbol for a trait bridge's `clear` entry point:
/// `{prefix}_clear_{trait_snake}`. Derived from the trait name, not from the configured
/// `clear_fn`. ~keep
pub fn trait_clear_symbol(prefix: &str, trait_name: &str) -> String {
    format!("{prefix}_clear_{}", trait_snake_of(trait_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trait_register_symbol_uses_the_configured_function_name() {
        assert_eq!(trait_register_symbol("demo", "register_ocr_backend"), "demo_register_ocr_backend");
        assert_eq!(trait_register_symbol("demo", "install_backend"), "demo_install_backend");
    }

    /// The register symbol comes from configuration, the unregister symbol from the trait name.
    /// A `register_fn` that does not happen to spell `register_{trait_snake}` is the row that
    /// proves the two are different facts. ~keep
    #[test]
    fn trait_register_symbol_is_not_derived_from_the_trait_name() {
        assert_ne!(
            trait_register_symbol("demo", "install_backend"),
            format!("demo_register_{}", trait_snake_of("OcrBackend"))
        );
    }

    #[test]
    fn trait_unregister_and_clear_symbols_snake_the_trait_name() {
        assert_eq!(trait_unregister_symbol("demo", "OcrBackend"), "demo_unregister_ocr_backend");
        assert_eq!(trait_clear_symbol("demo", "OcrBackend"), "demo_clear_ocr_backend");
        assert_eq!(trait_unregister_symbol("demo", "HTTPClient"), "demo_unregister_http_client");
    }
}
