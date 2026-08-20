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

    let mut owner = root;
    for segment in prefix {
        let Some(name) = segment_name(segment) else {
            return false;
        };
        match map.field_types.get(owner).and_then(|fields| fields.get(name)) {
            Some(next) => owner = next.as_str(),
            None => return false,
        }
    }

    let Some(name) = segment_name(last) else {
        return false;
    };
    map.enum_fields.get(owner).is_some_and(|fields| fields.contains(name))
}
