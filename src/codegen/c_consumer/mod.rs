//! Shared C-FFI consumer scaffolding for language backends.
//!
//! This module provides utilities for generating language bindings that consume
//! the C FFI layer produced by cbindgen. Each consumer backend (Go, Java, C#, Zig)
//! uses the same C interface:
//! - A C header file (`config.ffi_header_name()`)
//! - A library name (`config.ffi_lib_name()`)
//! - A symbol prefix (`config.ffi_prefix()`)
//! - Standard helper symbols: `{prefix}_free_string`, `{prefix}_last_error_code`, `{prefix}_last_error_context`
//!
//! It is also the single place that spells the *generated* C symbols — free functions,
//! opaque-type methods and streaming adapters. Both the FFI backend (which emits the
//! `#[unsafe(no_mangle)] extern "C"` items cbindgen turns into the header) and the docs
//! renderers (which must name symbols a reader can actually link against) derive their
//! names from these functions, so a rename cannot land on one side only.

use crate::codegen::naming::pascal_to_snake;
use crate::core::config::{ResolvedCrateConfig, resolve_output_dir};
use std::path::PathBuf;

/// Context capturing the shared FFI consumer inputs across all language backends.
pub struct CConsumerContext<'a> {
    /// Reference to the resolved crate configuration.
    pub config: &'a ResolvedCrateConfig,
    /// C header filename (e.g., "sample_markdown.h").
    pub header: String,
    /// C library name used for linking (e.g., "sample_markdown").
    pub lib_name: String,
    /// C symbol prefix for FFI functions (e.g., "htm").
    pub prefix: String,
}

impl<'a> CConsumerContext<'a> {
    /// Create a new CConsumerContext from the resolved crate configuration.
    pub fn from_config(config: &'a ResolvedCrateConfig) -> Self {
        Self {
            config,
            header: config.ffi_header_name(),
            lib_name: config.ffi_lib_name(),
            prefix: config.ffi_prefix(),
        }
    }
}

/// Return the C symbol name of a generated free function.
///
/// Format: `{prefix}_{function_snake}` — the shape `gen_free_function`
/// (`backends/ffi/gen_bindings/functions/orchestration.rs`) emits.
///
/// # The `prefix` contract
///
/// `prefix` is used **verbatim**; this function does not re-case it. Callers already hold it
/// in the spelling the emitted header uses — the FFI backend passes `config.ffi_prefix()`
/// straight through, and the docs renderers hold a PascalCase copy (`docs::generate_docs`)
/// which they snake-case on the way in. Re-casing here would silently re-spell an explicit
/// `[ffi] prefix` that carries an internal capital, and would do it differently from
/// `gen_cbindgen_toml`, which is the thing that actually writes the header.
pub fn free_function_symbol(prefix: &str, function_name: &str) -> String {
    format!("{prefix}_{}", pascal_to_snake(function_name))
}

/// Return the C symbol name of a method generated on an opaque type.
///
/// Format: `{prefix}_{type_snake}_{method_name}` — the shape `gen_method_wrapper` and
/// `gen_streaming_method_wrapper` (`backends/ffi/gen_bindings/functions/orchestration.rs`) emit.
///
/// The owning type is what separates this from [`free_function_symbol`]: a C symbol has no
/// namespace, so the type is folded into the name. Documenting a method as
/// `{prefix}_{method}` names a symbol that occurs zero times in the header.
///
/// The type component goes through `pascal_to_snake` (acronym-aware, matching the backend's
/// `c_symbol_component`) while the method component is used verbatim, because the backend
/// interpolates `method.name` — an already-snake_case Rust `fn` name — with no conversion.
/// See [`free_function_symbol`] for the `prefix` contract.
pub fn method_symbol(prefix: &str, type_name: &str, method_name: &str) -> String {
    format!("{prefix}_{}_{method_name}", pascal_to_snake(type_name))
}

/// Return the C symbol name of one operation of a generated streaming adapter.
///
/// Format: `{prefix}_{owner_snake}_{adapter_name}_{operation}`, where `operation` is
/// `start`, `next` or `free` — the shape `gen_stream_handle_functions`
/// (`backends/ffi/gen_bindings/helpers.rs`) emits.
///
/// See [`free_function_symbol`] for the `prefix` contract.
pub fn stream_adapter_symbol(prefix: &str, owner_type: &str, adapter_name: &str, operation: &str) -> String {
    format!("{}_{operation}", method_symbol(prefix, owner_type, adapter_name))
}

/// Return the C symbol name for freeing FFI-allocated strings.
///
/// Format: `{prefix}_free_string`
///
/// # Example
/// ```ignore
/// let sym = free_string_symbol("htm");
/// assert_eq!(sym, "htm_free_string");
/// ```
pub fn free_string_symbol(prefix: &str) -> String {
    format!("{prefix}_free_string")
}

/// Return the C symbol name for reading the thread-local last error code.
///
/// Format: `{prefix}_last_error_code`
///
/// # Example
/// ```ignore
/// let sym = last_error_code_symbol("krz");
/// assert_eq!(sym, "krz_last_error_code");
/// ```
pub fn last_error_code_symbol(prefix: &str) -> String {
    format!("{prefix}_last_error_code")
}

/// Return the C symbol name for reading the thread-local last error context message.
///
/// Format: `{prefix}_last_error_context`
///
/// # Example
/// ```ignore
/// let sym = last_error_context_symbol("krz");
/// assert_eq!(sym, "krz_last_error_context");
/// ```
pub fn last_error_context_symbol(prefix: &str) -> String {
    format!("{prefix}_last_error_context")
}

/// Resolve the per-backend output directory for generated files.
///
/// This helper wraps `resolve_output_dir` with a sensible default for C-FFI consumers,
/// allowing backends to pass a language-specific default (e.g., "packages/go/", "packages/java/src/main/java/").
///
/// # Arguments
/// - `config`: The Alef configuration.
/// - `default`: The backend-specific default output directory (e.g., "packages/go/").
///
/// # Returns
/// A PathBuf representing the resolved output directory.
pub fn default_output_dir(config: &ResolvedCrateConfig, default: &str) -> PathBuf {
    let resolved = resolve_output_dir(None, &config.name, default);
    PathBuf::from(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    fn make_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn free_function_symbol_produces_expected_format() {
        assert_eq!(free_function_symbol("htm", "parse_document"), "htm_parse_document");
    }

    #[test]
    fn method_symbol_folds_the_owning_type_into_the_name() {
        assert_eq!(
            method_symbol("literllm", "DefaultClient", "chat"),
            "literllm_default_client_chat"
        );
    }

    /// A C symbol has no namespace, so the owning type is the whole difference between a
    /// method and a same-named free function. Dropping it is the exact defect this helper
    /// exists to prevent: the docs used to publish `{prefix}_{method}`, which links against
    /// nothing.
    #[test]
    fn method_symbol_is_not_the_free_function_symbol() {
        assert_ne!(
            method_symbol("htm", "Converter", "convert"),
            free_function_symbol("htm", "convert"),
        );
    }

    #[test]
    fn stream_adapter_symbol_suffixes_the_operation() {
        assert_eq!(
            stream_adapter_symbol("literllm", "DefaultClient", "chat_stream", "start"),
            "literllm_default_client_chat_stream_start"
        );
        assert_eq!(
            stream_adapter_symbol("literllm", "DefaultClient", "chat_stream", "next"),
            "literllm_default_client_chat_stream_next"
        );
        assert_eq!(
            stream_adapter_symbol("literllm", "DefaultClient", "chat_stream", "free"),
            "literllm_default_client_chat_stream_free"
        );
    }

    /// The two type-component conversions in play must not drift.
    ///
    /// `method_symbol` uses `pascal_to_snake`, mirroring the FFI backend's
    /// `c_symbol_component`, which is what `gen_method_wrapper` applies. But
    /// `gen_stream_handle_functions` reaches the same component through heck's
    /// `to_snake_case` instead. The two agree today for every acronym shape below, which is
    /// why routing streaming through `method_symbol` is a no-op rename rather than a
    /// behaviour change -- this test is the thing that would fail if either conversion were
    /// ever changed independently, since the emitted symbol and the documented one would then
    /// come from different formulas.
    #[test]
    fn type_component_agrees_with_the_streaming_backends_heck_conversion() {
        use heck::ToSnakeCase;
        for type_name in [
            "DefaultClient",
            "Converter",
            "HTMLParser",
            "XMLHttpRequest",
            "IOError",
            "JSONLD",
            "UTF8Decoder",
            "Base64Encoder",
            "already_snake",
        ] {
            assert_eq!(
                method_symbol("p", type_name, "m"),
                format!("p_{}_m", type_name.to_snake_case()),
                "`{type_name}` snake-cases differently in the two backends"
            );
        }
    }

    #[test]
    fn free_string_symbol_produces_expected_format() {
        assert_eq!(free_string_symbol("htm"), "htm_free_string");
    }

    #[test]
    fn last_error_code_symbol_produces_expected_format() {
        assert_eq!(last_error_code_symbol("krz"), "krz_last_error_code");
    }

    #[test]
    fn last_error_context_symbol_produces_expected_format() {
        assert_eq!(last_error_context_symbol("krz"), "krz_last_error_context");
    }

    #[test]
    fn from_config_reads_ffi_fields() {
        let config = make_config();
        let ctx = CConsumerContext::from_config(&config);
        assert!(!ctx.header.is_empty());
        assert!(!ctx.lib_name.is_empty());
        assert!(!ctx.prefix.is_empty());
    }

    #[test]
    fn default_output_dir_uses_provided_default() {
        let config = make_config();
        let dir = default_output_dir(&config, "packages/go/");
        assert!(dir.to_string_lossy().contains("go"));
    }
}
