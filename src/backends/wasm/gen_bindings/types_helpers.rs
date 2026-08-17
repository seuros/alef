//! Shared helpers for WASM type generation.

use crate::core::ir::{ApiSurface, MethodDef, TypeRef};
use ahash::AHashSet;

/// Return a WASM binding surface whose struct fields and methods match the backend feature set.
///
/// The extractor can retain a cfg-gated field or method when the source crate was extracted with
/// a broader feature set than the WASM binding crate uses. Downstream struct, method and
/// conversion generators expect one coherent list, so drop inactive members and clear cfg markers
/// from active ones before generating WASM bindings.
///
/// Methods matter as much as fields here: `wasm_bindgen` emits one JS-visible export per method
/// with no Rust-cfg gating of its own, so a method left in place under a feature the binding
/// crate does not enable becomes a call into a core method that is not compiled. ~keep
pub(in crate::backends::wasm::gen_bindings) fn filter_cfg_fields_for_features(
    api: &ApiSurface,
    enabled_features: &[String],
) -> ApiSurface {
    let mut filtered = api.clone();
    for typ in &mut filtered.types {
        let mut fields = Vec::with_capacity(typ.fields.len());

        for mut field in std::mem::take(&mut typ.fields) {
            let Some(cfg) = field.cfg.as_deref() else {
                fields.push(field);
                continue;
            };

            if super::super::cfg::cfg_condition_enabled(cfg, enabled_features) {
                field.cfg = None;
                fields.push(field);
            }
        }

        typ.fields = fields;
        typ.methods = retain_enabled_methods(std::mem::take(&mut typ.methods), enabled_features);
    }
    for enum_def in &mut filtered.enums {
        enum_def.methods = retain_enabled_methods(std::mem::take(&mut enum_def.methods), enabled_features);
    }
    for error in &mut filtered.errors {
        error.methods = retain_enabled_methods(std::mem::take(&mut error.methods), enabled_features);
    }
    filtered
}

fn retain_enabled_methods(methods: Vec<MethodDef>, enabled_features: &[String]) -> Vec<MethodDef> {
    methods
        .into_iter()
        .filter_map(|mut method| {
            let Some(cfg) = method.cfg.as_deref() else {
                return Some(method);
            };
            if !super::super::cfg::cfg_condition_enabled(cfg, enabled_features) {
                return None;
            }
            method.cfg = None;
            Some(method)
        })
        .collect()
}

/// Returns `true` when `ty` is `Vec<Named>` where `Named` is a tagged-data enum.
pub(super) fn is_vec_of_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_data_enum_names.contains(n)))
}

/// Returns `true` when `ty` is a bare `Named` that is a tagged-data enum.
pub(super) fn is_bare_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Named(n) if tagged_data_enum_names.contains(n))
}

/// Returns `true` when `ty` is `Option<Named>` where `Named` is a tagged-data enum.
pub(super) fn is_option_of_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_data_enum_names.contains(n)))
}

/// Check if a TypeRef is a Copy type that shouldn't be cloned.
pub(super) fn is_copy_type(ty: &TypeRef, enum_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) => true,
        TypeRef::Duration => true,
        TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Path | TypeRef::Json => false,
        TypeRef::Optional(inner) => is_copy_type(inner, enum_names),
        TypeRef::Vec(_) | TypeRef::Map(_, _) => false,
        TypeRef::Named(n) => enum_names.contains(n),
        TypeRef::Unit => true,
    }
}

/// Extract the inner type of an `Optional` wrapper, or return the type itself.
pub(super) fn optional_inner(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}
