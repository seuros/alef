//! Derives enum-field classification from the crate's own IR instead of trusting a
//! hand-written `alef.toml` `fields_enum` list to have enumerated every enum-typed result
//! field.
//!
//! Before this module existed, `FieldResolver::is_enum` answered purely from the
//! author-declared `fields_enum` set (`E2eConfig::effective_fields_enum`). A consumer that
//! never populated `fields_enum` got `false` for every field, so the Rust e2e generator
//! emitted `<field>.to_string()` for enum-typed fields — a compile error whenever the enum
//! does not implement `Display` (only `Debug` is a safe assumption for an arbitrary enum).
//!
//! The fix has to be type-driven, not name-driven: the same crate can declare `kind: String`
//! on one struct and `kind: SomeEnum` on another, so a bare-field-name rule would misclassify
//! one of them regardless of which way it defaults. [`build_ir_enum_map`] therefore keys its
//! answer by `(owner_type, field_name)`, and [`is_enum_path`] only trusts that answer once it
//! has walked the field path from a known root type through the IR's own struct graph to the
//! exact type that owns the leaf segment.
use std::collections::{HashMap, HashSet};

use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::call_ir::named_type;

use super::parse::parse_path;
use super::types::{IrEnumMap, PathSegment};

/// Build the `(type, field) -> is-enum` / `(type, field) -> next type` maps [`IrEnumMap`]
/// needs, by inspecting every field of every `TypeDef` this crate declares.
///
/// A field's declared type resolves through [`named_type`] — the same `Option`/`Vec` unwrapper
/// `CallIr` already uses for parameter and return types (`Box<T>` fields carry the unboxed
/// named type directly in the IR, so no separate unwrap is needed for them). When the
/// resolved name matches a real `EnumDef`, the field is recorded as enum-typed on its owner.
/// When it instead matches another `TypeDef` — a struct the path can keep traversing into —
/// it is recorded as a traversal edge so multi-segment paths like `choices[0].finish_reason`
/// can advance their type cursor one segment at a time. A field whose resolved name is
/// neither (a primitive, or an external/opaque type the IR did not resolve) lands in neither
/// map, and a path through it answers `false` in [`is_enum_path`] — the same safe default an
/// unconfigured `fields_enum` entry already had.
pub(super) fn build_ir_enum_map(type_defs: &[TypeDef], enums: &[EnumDef]) -> IrEnumMap {
    let enum_names: HashSet<&str> = enums.iter().map(|e| e.name.as_str()).collect();
    let struct_names: HashSet<&str> = type_defs.iter().map(|t| t.name.as_str()).collect();

    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut enum_fields: HashMap<String, HashSet<String>> = HashMap::new();
    let mut enum_field_types: HashMap<String, HashMap<String, String>> = HashMap::new();

    for type_def in type_defs {
        for field in &type_def.fields {
            let Some(named) = named_type(&field.ty) else {
                continue;
            };
            if enum_names.contains(named) {
                enum_fields
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone());
                enum_field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            } else if struct_names.contains(named) {
                field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            }
        }
    }

    IrEnumMap {
        field_types,
        enum_fields,
        enum_field_types,
        root_type: None,
    }
}

/// The field/array/map-access name carried by a path segment, or `None` for `.length`/`.count`
/// pseudo-segments, which never name a real struct field.
fn segment_name(segment: &PathSegment) -> Option<&str> {
    match segment {
        PathSegment::Field(name) | PathSegment::ArrayField { name, .. } => Some(name),
        PathSegment::MapAccess { field, .. } => Some(field),
        PathSegment::Length => None,
    }
}

/// Walk `map.field_types` from `root` through `prefix`, returning the owner type the path's
/// last segment lands on — or `None` if any segment names something the IR does not recognize
/// as a field on the current owner. Shared by [`is_enum_path`] and [`enum_type_at_path`] so the
/// two answer from the exact same walk and can never disagree about which type a path reaches.
fn resolve_owner<'a>(map: &'a IrEnumMap, root: &'a str, prefix: &[PathSegment]) -> Option<&'a str> {
    let mut owner = root;
    for segment in prefix {
        let name = segment_name(segment)?;
        let next = map.field_types.get(owner).and_then(|fields| fields.get(name))?;
        owner = next.as_str();
    }
    Some(owner)
}

/// Walk `path` from `map.root_type` through `map.field_types`, answering whether the leaf
/// segment's declared type (per [`build_ir_enum_map`]) is a real IR enum.
///
/// Returns `false` — never "unknown" — whenever the root type is unresolved, a segment names
/// something the IR does not recognize as a field on the current owner type, or `map` was
/// never populated. Every one of those is the pre-existing behaviour for a field with no
/// `fields_enum` entry, so this is purely additive: it can only turn a `false` into a `true`
/// when the IR positively confirms the leaf is enum-typed on the exact type the path reaches.
pub(super) fn is_enum_path(map: &IrEnumMap, path: &str) -> bool {
    let Some(root) = map.root_type.as_deref() else {
        return false;
    };
    let segments = parse_path(path);
    let Some((last, prefix)) = segments.split_last() else {
        return false;
    };
    let Some(owner) = resolve_owner(map, root, prefix) else {
        return false;
    };
    let Some(name) = segment_name(last) else {
        return false;
    };
    map.enum_fields.get(owner).is_some_and(|fields| fields.contains(name))
}

/// Resolve the concrete IR enum type name backing `path`'s leaf segment, walking the same
/// `map.field_types` chain as [`is_enum_path`]. Returns `None` under the exact same
/// "unknown" conditions `is_enum_path` returns `false` for; callers that need to know *which*
/// enum a positively-classified field resolves to (not just that it is one) use this instead
/// of re-walking the path themselves.
pub(super) fn enum_type_at_path(map: &IrEnumMap, path: &str) -> Option<String> {
    let root = map.root_type.as_deref()?;
    let segments = parse_path(path);
    let (last, prefix) = segments.split_last()?;
    let owner = resolve_owner(map, root, prefix)?;
    let name = segment_name(last)?;
    map.enum_field_types
        .get(owner)
        .and_then(|fields| fields.get(name))
        .cloned()
}
