//! Bind a trait-bridge registry fixture to the C ABI symbol the FFI backend really exports.
//!
//! `src/backends/ffi/trait_bridge/registration.rs` derives `{prefix}_unregister_{trait_snake}`
//! and `{prefix}_clear_{trait_snake}` from the bridge's trait name and discards the
//! `unregister_fn`/`clear_fn` config text's spelling, so a crate that names
//! `clear_fn = "clear_ocr_backends"` on a trait `OcrBackend` ships `..._clear_ocr_backend`.
//! `recipe::trait_bridge_derived_c_identity` is where that derivation is re-stated for
//! consumers; this module is what makes the C emitter obey it.

use super::ResolvedCallInfo;
use crate::core::config::e2e::CallConfig;
use crate::e2e::codegen::recipe::TraitBridgeRegistryOperation;

/// Overwrite `info`'s function name (and, for `unregister`/`clear`, its out-param) with the
/// derived C ABI identity.
///
/// The derived identity is what the FFI backend exports, so the only config that may outrank
/// it is the one statement that is *about* C: `overrides.c.function`. The base
/// `[crates.e2e.calls.*] function` is the Rust core's name, shared by every language, and a
/// well-formed config populates it -- gating this on `info.function_name` being empty
/// therefore let the Rust spelling shadow the ABI truth for every properly configured
/// registry fixture, emitting a symbol the generated header never declares. ~keep
pub(super) fn apply(
    info: &mut ResolvedCallInfo,
    call: &CallConfig,
    lang: &str,
    identity: Option<(TraitBridgeRegistryOperation, String)>,
) {
    if call
        .overrides
        .get(lang)
        .and_then(|override_config| override_config.function.as_deref())
        .is_some_and(|function| !function.trim().is_empty())
    {
        return;
    }
    let Some((operation, derived_name)) = identity else {
        return;
    };
    info.function_name = derived_name;
    // `unregister`/`clear` C exports always take a trailing `out_error` out-param that the
    // shared, language-agnostic `[crates.e2e.calls.*]` args config has no way to express
    // (other bindings surface it via an exception/error-return mechanism instead).
    // `register` needs no such treatment here: register-shaped fixtures require vtable/
    // user_data wiring this generic void-call fallback does not build, so they never reach
    // this branch as a `returns_void` call in practice. See `unregister_fn.jinja` /
    // `clear_fn.jinja` for the ABI shapes. ~keep
    if matches!(
        operation,
        TraitBridgeRegistryOperation::Unregister | TraitBridgeRegistryOperation::Clear
    ) {
        info.extra_args.push("NULL".to_string());
    }
}
