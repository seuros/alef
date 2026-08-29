//! Small pure helpers that re-derive Go binding-backend signature decisions from the same
//! public IR facts the backend itself reads, so `snippet.rs`'s docs-snippet rendering agrees
//! with the generated bindings instead of re-asserting an independent copy of the logic.

use crate::core::ir::{ParamDef, TypeRef};
use std::collections::HashSet;

/// Unwraps a (possibly `Optional`-wrapped) `TypeRef::Named` down to its type name.
///
/// Used to match a function parameter's IR type against the configured `options_type`
/// name so the pointer-vs-value and arity decisions below can be derived from the
/// same signature the Go binding backend generated from, instead of re-asserting it
/// independently. ~keep
pub(super) fn go_ir_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => go_ir_named_type(inner),
        _ => None,
    }
}

/// Mirrors `backends::go::gen_bindings::functions::gen_function_wrapper`'s pointer-vs-value
/// decision for a non-bridge parameter: the Go binding backend emits `*T` when the IR
/// parameter is `optional`, or when its `Named` type is opaque — value `T` otherwise. Both
/// this function and the binding backend read the same `ParamDef`/`TypeDef.is_opaque`
/// facts; this is a re-derivation of the same public inputs; it is not a copy of any
/// gen_bindings-private logic. ~keep
pub(super) fn go_options_param_is_pointer(param: &ParamDef, opaque_names: &HashSet<&str>) -> bool {
    if param.optional {
        return true;
    }
    matches!(&param.ty, TypeRef::Named(name) if opaque_names.contains(name.as_str()))
}

/// Mirrors `gen_bindings::functions::is_bridge_param`'s two membership checks (by
/// parameter name, then by `Named` type alias) using the same `TraitBridgeConfig` facts
/// the binding backend reads — the params those checks match are real Rust function
/// parameters that the Go binding backend strips from its emitted signature (replaced by
/// a `nil` argument at the FFI call site), so they must not be counted toward the
/// Go-visible arity used by the `extra_args` clamp below. ~keep
pub(super) fn go_is_bridge_param(
    param: &ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    if bridge_param_names.contains(&param.name) {
        return true;
    }
    go_ir_named_type(&param.ty).is_some_and(|name| bridge_type_aliases.contains(name))
}
