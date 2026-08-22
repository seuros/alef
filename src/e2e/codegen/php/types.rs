//! PHP e2e helper types and type-classification helpers.
//! These utilities were previously defined in `php.rs` during extractor phase.

use super::enum_variant_access::PhpEnumLowering;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, TypeRef};
use crate::e2e::field_access::PhpGetterMap;
use std::collections::{HashMap, HashSet};

/// Build a per-`(owner_type, field_name)` PHP getter classification plus chain-resolution
/// metadata from the IR's `TypeDef`s and `EnumDef`s.
///
/// For each type, marks fields as needing getter syntax when their mapped Rust type is
/// non-scalar in PHP (`Named` struct, `Vec<Named>`, `Map`, `Json`, `Bytes`, a data enum).
/// Also records each field's referenced `Named` inner type so the resolver can advance
/// the current-type cursor as it walks multi-segment paths like `outer.inner.content`.
///
/// ~keep The scalar question is answered by [`crate::backends::php::is_php_prop_scalar`] — the
/// binding backend's own predicate — rather than by a copy of it, and it is handed exactly the
/// `enum_names` set the backend hands it: only the enums PHP lowers to a plain `string`. A
/// local copy taking ALL enum names used to live here, which classified a tagged data enum
/// field as a `#[php(prop)]` scalar and emitted `->format` for a field the binding exposes only
/// as `getFormat()`.
///
/// Enums the backend lowers to a flat `#[php_class]` are registered as owner types too, so a
/// path can keep walking into a variant payload. Their payload properties are `#[php(getter)]`
/// backed, which ext-php-rs registers as read-only PHP *properties*, so they are recorded in
/// `all_fields` but deliberately NOT in `getters`.
///
/// `root_type` is derived (best-effort) from a `result_type` override on any backend
/// (`c`, `csharp`, `java`, `kotlin`, `go`, `php`) and otherwise inferred by matching
/// `result_fields` against `TypeDef.fields`. When no root can be determined, chain
/// resolution falls back to the legacy bare-name union (sound only when no field names
/// collide across types).
pub(super) fn build_php_getter_map(
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[EnumDef],
    call: &CallConfig,
    result_fields: &HashSet<String>,
) -> PhpGetterMap {
    let lowering = PhpEnumLowering::from_enums(enums);
    let prop_scalar_enums = lowering.php_prop_scalar_enum_names();
    let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut all_fields: HashMap<String, HashSet<String>> = HashMap::new();
    for td in type_defs {
        let mut getter_fields: HashSet<String> = HashSet::new();
        let mut field_type_map: HashMap<String, String> = HashMap::new();
        let mut td_all_fields: HashSet<String> = HashSet::new();
        for f in &td.fields {
            td_all_fields.insert(f.name.clone());
            if !crate::backends::php::is_php_prop_scalar(&f.ty, &prop_scalar_enums) {
                getter_fields.insert(f.name.clone());
            }
            if let Some(named) = inner_named(&f.ty) {
                field_type_map.insert(f.name.clone(), named);
            }
        }
        getters.insert(td.name.clone(), getter_fields);
        all_fields.insert(td.name.clone(), td_all_fields);
        if !field_type_map.is_empty() {
            field_types.insert(td.name.clone(), field_type_map);
        }
    }
    for enum_def in enums {
        let Some(properties) = lowering.flat_class_properties(enum_def) else {
            continue;
        };
        let mut field_type_map: HashMap<String, String> = HashMap::new();
        let mut enum_all_fields: HashSet<String> = HashSet::new();
        for property in properties {
            if let Some(payload) = property.payload_type {
                field_type_map.insert(property.name.clone(), payload);
            }
            enum_all_fields.insert(property.name);
        }
        getters.insert(enum_def.name.clone(), HashSet::new());
        all_fields.insert(enum_def.name.clone(), enum_all_fields);
        if !field_type_map.is_empty() {
            field_types.insert(enum_def.name.clone(), field_type_map);
        }
    }
    let root_type = derive_root_type(call, type_defs, result_fields);
    PhpGetterMap {
        getters,
        field_types,
        root_type,
        all_fields,
    }
}

/// Unwrap `Option<T>` / `Vec<T>` to the innermost `Named` type name, if any.
/// Returns `None` for primitives, scalars, `Map`, `Json`, `Bytes`, and `Unit`.
pub(super) fn inner_named(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(n) => Some(n.clone()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
        _ => None,
    }
}

/// Derive the IR type name backing the result variable in PHP-generated assertions.
///
/// Lookup order:
/// 1. `call.overrides[<lang>]`.result_type for any of `php`, `c`, `csharp`,
///    `java`, `kotlin`, `go` (first non-empty wins).
/// 2. Type-defs whose field names form a superset of `result_fields` (when exactly
///    one matches).
///
/// Returns `None` when neither yields a definitive answer; callers fall back to the
/// legacy bare-name union behaviour.
pub(super) fn derive_root_type(
    call: &CallConfig,
    type_defs: &[crate::core::ir::TypeDef],
    result_fields: &HashSet<String>,
) -> Option<String> {
    const LOOKUP_LANGS: &[&str] = &["php", "c", "csharp", "java", "kotlin", "go"];
    for lang in LOOKUP_LANGS {
        if let Some(o) = call.overrides.get(*lang)
            && let Some(rt) = o.result_type.as_deref()
            && !rt.is_empty()
            && type_defs.iter().any(|td| td.name == rt)
        {
            return Some(rt.to_string());
        }
    }
    if result_fields.is_empty() {
        return None;
    }
    let matches: Vec<&crate::core::ir::TypeDef> = type_defs
        .iter()
        .filter(|td| {
            let names: HashSet<&str> = td.fields.iter().map(|f| f.name.as_str()).collect();
            result_fields.iter().all(|rf| names.contains(rf.as_str()))
        })
        .collect();
    if matches.len() == 1 {
        return Some(matches[0].name.clone());
    }
    None
}
