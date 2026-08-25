use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use ahash::AHashSet;

/// Free-function analogue of [`crate::codegen::shared::can_auto_delegate_with_named_let_bindings`].
///
/// [`crate::codegen::shared::can_auto_delegate_function`] additionally rejects every non-opaque
/// `&Named`, `&[Named]` and `&[&str]` param, because most backends' call-arg builders only know
/// how to `.into()` an *owned* value. That restriction does not apply to a generator whose
/// delegation body pairs [`super::gen_named_let_bindings_no_promote`] (or
/// [`super::gen_named_let_bindings_pub`]) with [`super::gen_call_args_with_let_bindings`]: those
/// two emit an owned `{name}_core` binding and pass `&{name}_core`, which is exactly the borrow
/// the core signature wants. Without this relaxation such functions fall through to the backend's
/// unimplemented body, which for a non-fallible return emits `compile_error!` into the consumer's
/// default build path. Only generators wiring up that let-binding pair may use this. ~keep
pub fn can_auto_delegate_function_with_named_let_bindings(func: &FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && crate::codegen::shared::is_delegatable_return(&func.return_type)
}

/// Check if params contain any non-opaque Named types that need let bindings.
/// This includes direct Named types, `Vec<Named>` types, `Vec<String>` params
/// with is_ref=true (which need a `Vec<&str>` intermediate to pass as `&[&str]`),
/// and sanitized `Vec<String>` params (which are JSON-deserialized to tuples).
pub fn has_named_params(params: &[ParamDef], opaque_types: &AHashSet<String>) -> bool {
    params.iter().any(|p| match &p.ty {
        TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => true,
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name.as_str()))
                || (matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref)
                || (matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some())
        }
        _ => false,
    })
}

/// Check if a param type is safe for non-opaque delegation (no complex conversions needed).
/// Vec and Map params can cause type mismatches (e.g. `Vec<String>` vs `&[&str]`).
///
/// `Json` is delegatable: the binding takes a JSON string and `gen_call_args` emits
/// `serde_json::from_str(...)` to bridge it into the core `serde_json::Value` parameter.
/// This lets fluent-builder methods like `with_extension(self, key: String, value: Value) -> Self`
/// be auto-generated instead of being rejected as non-delegatable.
pub fn is_simple_non_opaque_param(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Optional(inner) => is_simple_non_opaque_param(inner),
        _ => false,
    }
}
