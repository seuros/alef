//! Derives collection (`Vec<T>`)-field classification from the crate's own IR instead of
//! trusting a hand-written `alef.toml` `fields_array`/`fields_optional` list to have named
//! every collection-typed result field.
//!
//! Before this module existed, `FieldResolver::is_array`/`is_collection_root` answered purely
//! from `array_fields`/`optional_fields` (`E2eConfig::effective_fields_array`/
//! `effective_fields_optional`). Those sets are populated from element-traversal paths like
//! `choices[0].message` — they tell a resolver a field IS a collection only when the operator
//! also declared how one of its elements is accessed. A bare collection field with no per-
//! element path in the fixture suite at all (e.g. a recursive `List<DataNode> Children` field
//! nothing ever indexes into) has no config signal whatsoever, so `is_collection_root` answered
//! `false` for it — the same gap `ir_enum` closed for enum-typed fields, but for `Vec<T>`.
//!
//! The fix has to be type-driven, not name-driven, for the identical reason `ir_enum` is:
//! the same crate can declare `items: Vec<T>` on one struct and `items: String` on another, so
//! a bare-field-name rule would misclassify one of them regardless of which way it defaults.
//! [`build_ir_collection_map`] therefore keys its answer by `(owner_type, field_name)`, and
//! [`is_collection_path`] only trusts that answer once it has walked the field path from a
//! known root type through the IR's own struct graph to the exact type that owns the leaf
//! segment — mirroring `ir_enum::is_enum_path` exactly.
use std::collections::{HashMap, HashSet};

use crate::core::ir::TypeDef;
use crate::e2e::codegen::call_ir::named_type;

use super::parse::{parse_path, segment_name};
use super::types::IrCollectionMap;

/// Build the `(type, field) -> is-Vec` / `(type, field) -> next type` maps [`IrCollectionMap`]
/// needs, by inspecting every field of every `TypeDef` this crate declares.
///
/// A field is recorded as collection-typed on its owner when its declared type is `Vec<T>`
/// (`Option<Vec<T>>` counts too — the FFI/binding layer already collapses "absent" into an
/// empty/null collection, so optionality must not hide the field's real shape). A field whose
/// [`named_type`]-resolved name matches another `TypeDef` is additionally recorded as a
/// traversal edge, exactly as `ir_enum::build_ir_enum_map` records struct-to-struct edges, so a
/// multi-segment path like `parent.children` can advance its type cursor one segment at a time
/// before answering the collection question at the leaf.
pub(super) fn build_ir_collection_map(type_defs: &[TypeDef]) -> IrCollectionMap {
    let struct_names: HashSet<&str> = type_defs.iter().map(|t| t.name.as_str()).collect();

    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut collection_fields: HashMap<String, HashSet<String>> = HashMap::new();

    for type_def in type_defs {
        for field in &type_def.fields {
            if is_vec_type(&field.ty) {
                collection_fields
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone());
            }
            let Some(named) = named_type(&field.ty) else {
                continue;
            };
            if struct_names.contains(named) {
                field_types
                    .entry(type_def.name.clone())
                    .or_default()
                    .insert(field.name.clone(), named.to_string());
            }
        }
    }

    IrCollectionMap {
        field_types,
        collection_fields,
        root_type: None,
    }
}

/// `true` when `ty` is `Vec<T>`, seeing through an `Option` wrapper the same way
/// `named_type` does.
///
/// `pub(super)` rather than private: [`super::ir_enum::build_variant_payload_types`] reuses this
/// exact check to record whether a tagged-union variant's single payload field is itself a
/// collection (`Variant(Vec<Item>)`), rather than a struct wrapping one — the same "is this a
/// `Vec`, seeing through `Option`" question, answered once. ~keep
pub(super) fn is_vec_type(ty: &crate::core::ir::TypeRef) -> bool {
    match ty {
        crate::core::ir::TypeRef::Vec(_) => true,
        crate::core::ir::TypeRef::Optional(inner) => is_vec_type(inner),
        _ => false,
    }
}

/// Walk `path` from `map.root_type` through `map.field_types`, answering whether the leaf
/// segment's declared type (per [`build_ir_collection_map`]) is a real `Vec<T>`.
///
/// Returns `false` — never "unknown" — whenever the root type is unresolved, a segment names
/// something the IR does not recognize as a field on the current owner type, or `map` was
/// never populated. Every one of those is the pre-existing behaviour for a field with no
/// collection config entry, so this is purely additive: it can only turn a `false` into a
/// `true` when the IR positively confirms the leaf is `Vec`-typed on the exact type the path
/// reaches. Mirrors `ir_enum::is_enum_path` exactly.
pub(super) fn is_collection_path(map: &IrCollectionMap, path: &str) -> bool {
    let Some(root) = map.root_type.as_deref() else {
        return false;
    };
    is_collection_path_from(map, root, path)
}

/// Walk `path` from an explicitly resolved IR owner type rather than the call's result root.
/// Tagged-union renderers use this after narrowing a variant to its payload type, since a
/// struct-only walk from `root_type` cannot cross the enum boundary itself. ~keep
pub(super) fn is_collection_path_from(map: &IrCollectionMap, root: &str, path: &str) -> bool {
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
    map.collection_fields
        .get(owner)
        .is_some_and(|fields| fields.contains(name))
}

/// The IR type name `path`'s elements are, walking `map.field_types` from `map.root_type`
/// through EVERY segment of `path` (including the leaf) — e.g. `"tables"` on a `Vec<Table>`
/// field resolves to `"Table"`, because [`build_ir_collection_map`] records a `Vec<T>` field's
/// traversal edge as `T`, the same way it would a plain struct-to-struct edge.
///
/// `None` under the same "IR cannot judge" conditions [`is_collection_path`] answers `false`
/// for: an unresolved root, or a segment the IR does not recognize as a struct-to-struct edge on
/// the type reached so far (a scalar leaf, a foreign type, a field not populated here because
/// its own type is not itself another struct in `type_defs`). Callers validating an `Iterate`
/// operation's per-item fields against this answer must treat `None` as "no answer, don't
/// reject" — exactly the same fallback every other IR oracle in this module uses.
pub(super) fn element_type_at_path(map: &IrCollectionMap, path: &str) -> Option<String> {
    let root = map.root_type.as_deref()?;
    let segments = parse_path(path);
    let mut owner = root;
    for segment in &segments {
        let name = segment_name(segment)?;
        owner = map.field_types.get(owner)?.get(name)?.as_str();
    }
    Some(owner.to_string())
}

#[cfg(test)]
mod element_type_at_path_tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    /// `Container { rows: Vec<Row> }`, `Row { values: Vec<String> }` — the shape a fixture's
    /// `iterate` operation over a collection field needs resolved.
    fn type_defs() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "Container".to_string(),
                fields: vec![field("rows", TypeRef::Vec(Box::new(TypeRef::Named("Row".to_string()))))],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Row".to_string(),
                fields: vec![field("values", TypeRef::Vec(Box::new(TypeRef::String)))],
                ..TypeDef::default()
            },
        ]
    }

    fn anchored_map() -> IrCollectionMap {
        let mut map = build_ir_collection_map(&type_defs());
        map.root_type = Some("Container".to_string());
        map
    }

    #[test]
    fn a_vec_field_resolves_to_its_element_type() {
        assert_eq!(element_type_at_path(&anchored_map(), "rows"), Some("Row".to_string()));
    }

    #[test]
    fn an_indexed_vec_field_resolves_the_same_way() {
        assert_eq!(
            element_type_at_path(&anchored_map(), "rows[0]"),
            Some("Row".to_string())
        );
    }

    #[test]
    fn an_unknown_field_resolves_to_nothing() {
        assert_eq!(element_type_at_path(&anchored_map(), "not_a_real_field"), None);
    }

    #[test]
    fn no_anchored_root_resolves_to_nothing() {
        let map = build_ir_collection_map(&type_defs());
        assert_eq!(element_type_at_path(&map, "rows"), None);
    }
}
