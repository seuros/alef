//! The single seam that decides whether a C argument crosses as an opaque `AlefHandle` or as a
//! real pointer -- the "none" sentinel for an omitted optional argument, and the handle-versus-
//! JSON-string lowering for a present one.
//!
//! The C ABI represents every opaque/named type (`TypeRef::Named`, bare or `Optional<_>`) as
//! the scalar generational handle `AlefHandle` (`typedef uint64_t {PREFIX}AlefHandle`) -- see
//! `src/backends/ffi/type_map.rs::c_param_optional`/`c_return_optional`. An absent optional
//! argument of that kind must therefore use `0` as its "none" sentinel, matching the FFI bridge
//! codegen's own convention (`src/backends/ffi/gen_bindings/helpers.rs::ffi_null_return_value`,
//! `Some("AlefHandle") => "0"`). Every other arg kind (`string`, `mock_url`, `bytes`, ...) is a
//! genuine C pointer (`const char *`, `void *`) and keeps the `NULL` sentinel.
//!
//! Three call sites in this backend render an omitted optional argument: the free-function/
//! typed-arg path (`c/assertions.rs::build_args_string_c`), the doc-snippet path
//! (`c/test_function.rs::render_snippet_body`), and the client-method path
//! (`c/test_function.rs`'s `client_factory` branch, which every one of the main e2e-test-file
//! emitter, the doc-snippet emitter, and the test-app emitter share). Before this module
//! existed, the client-method path never consulted the core IR at all -- it decided the
//! sentinel purely from the fixture's configured `arg_type` label, which defaults to `"string"`
//! (`crate::core::config::e2e::defaults::default_arg_type`) whenever a fixture author never set
//! it explicitly. A handle-typed parameter with no configured `arg_type` therefore rendered as
//! `NULL`, which does not compile against the `AlefHandle` (`unsigned long long`) parameter the
//! FFI header actually declares. Routing every call site through [`resolve_optional_sentinel`]
//! -- which checks the IR's declared parameter type first, falling back to the configured label
//! only when no signature is available -- is the fix: one seam answering "what is the null
//! sentinel for this C type", so the three call sites cannot disagree again.
//!
//! [`ir_declares_a_json_string_param`] asks the same question of a *present* argument, and it
//! lives here so it is answered from the same fact ([`handle_param_type_name`]) rather than from
//! a second, drifting rule.

use crate::core::ir::TypeRef;
use crate::e2e::codegen::call_ir::TargetParams;

#[cfg(test)]
mod batch_input_regression_tests;

/// Sentinel for an arg described only by its configured `arg_type` label (`"handle"` /
/// `"json_object"` cross as `AlefHandle`; everything else is a real pointer). Used as the
/// fallback when no IR signature is available to check the parameter's declared type directly.
pub(super) fn c_optional_sentinel(arg_type: &str) -> &'static str {
    if matches!(arg_type, "json_object" | "handle") {
        "0"
    } else {
        "NULL"
    }
}

/// The IR type name a C parameter carries as an opaque `AlefHandle` rather than as a literal.
///
/// Mirrors `backends::ffi::type_map::c_param_type_with_paths_and_enums`, the mapper that
/// actually spells the exported header: a bare `Named` and an `Optional<Named>` cross the C ABI
/// as `AlefHandle`, and nothing else does. `Vec<Named>` and `Map<_, Named>` cross as a JSON
/// `*const c_char`, so this deliberately does NOT unwrap through them the way `c.rs`'s
/// `named_type` does for the `element_type` backfill -- unwrapping there would reject arguments
/// whose JSON string literal is exactly what the parameter wants. ~keep
pub(super) fn handle_param_type_name(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Whether the core IR proves the parameter an `args` entry fills crosses the C ABI as a JSON
/// `const char *` rather than as an opaque `AlefHandle`.
///
/// `arg_type = "json_object"` describes the *fixture value* (an object, or an array of them), not
/// the parameter's C shape, and the two do not coincide. `backends::ffi::type_map` exports only a
/// bare `Named` and an `Optional<Named>` as `AlefHandle`; `Vec<_>`, `Map<_, _>`, `Json`, `Bytes`
/// and `Duration` all cross as `*const c_char`, i.e. the serialized JSON text. So a batch
/// parameter (`Vec<ItemInput>`) is handed the JSON array string, and building a handle for it is
/// an `incompatible integer to pointer conversion` against the very header alef generated -- which
/// is what `c/test_function.rs::build_json_object_arg_handles` did, because `c.rs`'s
/// `element_type` backfill runs through `named_type`, and `named_type` unwraps `Vec` to its
/// element. The declared parameter is the authority, never the unwrapped element name.
///
/// Only [`TargetParams::Known`] licenses the claim: `IrAbsent` and `Unresolvable` learned nothing
/// about the target, so they keep the pre-existing construct-a-handle behaviour rather than read
/// silence as proof of a string parameter -- the same asymmetry [`resolve_optional_sentinel`]
/// encodes. ~keep
pub(super) fn ir_declares_a_json_string_param(target_params: TargetParams<'_>, arg_name: &str, index: usize) -> bool {
    target_params
        .param_for(arg_name, index)
        .is_some_and(|param| handle_param_type_name(&param.ty).is_none())
}

/// Resolve the "none" sentinel for an omitted optional argument, preferring the IR's declared
/// parameter type over the fixture's configured `arg_type` label whenever a signature is
/// available.
///
/// `target_params` licenses the claim: when the IR resolved the target's parameters and the one
/// this `args` entry fills is a handle-crossing type ([`handle_param_type_name`]), the sentinel
/// is `0` regardless of what `arg_type` says -- an unconfigured `arg_type` defaults to
/// `"string"` and must not override what the IR proves about the actual parameter. When the IR
/// learned nothing (`TargetParams::IrAbsent`/`Unresolvable`) or the parameter did not resolve,
/// this falls back to [`c_optional_sentinel`]'s config-only answer, exactly the pre-existing
/// behavior for callers with no signature to check.
pub(super) fn resolve_optional_sentinel(
    target_params: TargetParams<'_>,
    arg_name: &str,
    index: usize,
    arg_type: &str,
) -> &'static str {
    if let Some(param) = target_params.param_for(arg_name, index)
        && handle_param_type_name(&param.ty).is_some()
    {
        return "0";
    }
    c_optional_sentinel(arg_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ParamDef;

    fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional,
            ..ParamDef::default()
        }
    }

    #[test]
    fn config_only_sentinel_treats_handle_and_json_object_as_zero() {
        assert_eq!(c_optional_sentinel("handle"), "0");
        assert_eq!(c_optional_sentinel("json_object"), "0");
        assert_eq!(c_optional_sentinel("string"), "NULL");
        assert_eq!(c_optional_sentinel("mock_url"), "NULL");
    }

    #[test]
    fn ir_declared_handle_param_wins_over_default_string_arg_type() {
        // Regression: a fixture that never configured `arg_type` for a handle-typed
        // optional parameter used to render `NULL` (the config-only default), which does
        // not compile against the `AlefHandle` (`unsigned long long`) parameter the FFI
        // header declares. The IR must win when it is available.
        let params = [param(
            "cursor",
            TypeRef::Optional(Box::new(TypeRef::Named("Cursor".into()))),
            true,
        )];
        let target_params = TargetParams::Known(&params);
        assert_eq!(
            resolve_optional_sentinel(target_params, "cursor", 0, "string"),
            "0",
            "an IR-declared handle parameter must use `0` even when `arg_type` defaulted to \"string\""
        );
    }

    #[test]
    fn ir_declared_string_param_keeps_null() {
        let params = [param("after", TypeRef::Optional(Box::new(TypeRef::String)), true)];
        let target_params = TargetParams::Known(&params);
        assert_eq!(resolve_optional_sentinel(target_params, "after", 0, "string"), "NULL");
    }

    #[test]
    fn ir_absent_falls_back_to_config_label() {
        assert_eq!(
            resolve_optional_sentinel(TargetParams::IrAbsent, "cursor", 0, "handle"),
            "0"
        );
        assert_eq!(
            resolve_optional_sentinel(TargetParams::IrAbsent, "cursor", 0, "string"),
            "NULL"
        );
    }

    #[test]
    fn unresolvable_target_falls_back_to_config_label() {
        assert_eq!(
            resolve_optional_sentinel(TargetParams::Unresolvable, "cursor", 0, "handle"),
            "0"
        );
    }
}
