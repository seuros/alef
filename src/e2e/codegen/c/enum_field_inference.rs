//! IR-derived `fields_c_types` inference for enum-typed leaf fields.
//!
//! `[crates.e2e.fields_c_types]` is how an operator tells the C e2e generator that a leaf
//! accessor returns something other than `char*`. A field the operator never declared falls
//! through to `emit_nested_accessor`'s leaf arm default: `char* {local} = {accessor}(...)`,
//! which only compiles when the accessor genuinely returns a C string. For a genuinely
//! enum-typed field (e.g. `DataNode.kind: DataNodeKind`) the FFI accessor instead returns an
//! opaque `AlefHandle` requiring a further `_to_string()` call — a mismatch the C compiler
//! reports as "incompatible integer to pointer conversion", not something `alef e2e generate`
//! catches at generation time, because nothing upstream consulted the field's actual IR type
//! before guessing `char*`. This module closes that gap by deriving the missing
//! `fields_c_types` entries straight from the IR, so a config declaration is no longer required
//! for a field the IR already fully describes.

use heck::ToSnakeCase;
use std::collections::HashMap;

/// Peel `Optional` wrappers down to the inner type reference. A C leaf accessor is emitted
/// identically for `DataNodeKind` and `Option<DataNodeKind>` — the FFI already collapses
/// "absent" into a sentinel handle/`NULL` — so optionality must not hide a field's real shape
/// from the enum inference below.
fn peel_optional(ty: &crate::core::ir::TypeRef) -> &crate::core::ir::TypeRef {
    match ty {
        crate::core::ir::TypeRef::Optional(inner) => peel_optional(inner),
        other => other,
    }
}

/// Derive `fields_c_types`-shaped entries directly from the IR struct definitions, for every
/// field whose declared Rust type names a real registered enum.
///
/// Returned entries are proposals, not the sole source of truth: the caller unions them under
/// any operator-declared entry via `HashMap::entry().or_insert()`, so an explicit
/// `fields_c_types` override always wins. This is one layer earlier than
/// `super::enum_fields_from_ir`: that function trusts the IR only for the *enum-ness* of a
/// field config already declares; this one trusts the IR for the declaration itself when
/// config has none. ~keep
pub(super) fn enum_fields_c_types_from_ir(
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> HashMap<String, String> {
    let mut derived = HashMap::new();
    for type_def in type_defs {
        let parent_snake = type_def.name.to_snake_case();
        for field in &type_def.fields {
            let crate::core::ir::TypeRef::Named(type_name) = peel_optional(&field.ty) else {
                continue;
            };
            if !enums.iter().any(|e| &e.name == type_name) {
                continue;
            }
            derived.insert(
                format!("{parent_snake}.{}", field.name.to_snake_case()),
                type_name.clone(),
            );
        }
    }
    derived
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::c::{enum_fields_from_ir, try_emit_enum_accessor};
    use std::collections::HashSet;

    /// Regression for the tslp `test_data_extraction.c` compile failure: `DataNode.kind` is a
    /// real `DataNodeKind` enum field, but `[crates.e2e.fields_c_types]` never declared
    /// `"data_node.kind"`. Without IR-derived inference, `emit_nested_accessor`'s leaf arm fell
    /// through to the default `char* data_kind = ts_pack_data_node_kind(data_handle);` even
    /// though the accessor returns an opaque `AlefHandle`, and gcc rejected the assignment as an
    /// "incompatible integer to pointer conversion". `enum_fields_c_types_from_ir` must recover
    /// the missing entry directly from the IR field's declared type, with no config at all. ~keep
    #[test]
    fn enum_fields_c_types_from_ir_recovers_an_undeclared_enum_leaf_field() {
        let enums = vec![crate::core::ir::EnumDef {
            name: "DataNodeKind".into(),
            ..crate::core::ir::EnumDef::default()
        }];
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "DataNode".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: crate::core::ir::TypeRef::Named("DataNodeKind".into()),
                ..crate::core::ir::FieldDef::default()
            }],
            ..crate::core::ir::TypeDef::default()
        }];

        let derived = enum_fields_c_types_from_ir(&type_defs, &enums);

        assert_eq!(
            derived.get("data_node.kind").map(String::as_str),
            Some("DataNodeKind"),
            "got: {derived:?}"
        );
    }

    /// `Option<DataNodeKind>` must be recognized the same as a bare `DataNodeKind` field: the
    /// FFI accessor returns the same sentinel-handle shape either way, so optionality must not
    /// hide the field from inference.
    #[test]
    fn enum_fields_c_types_from_ir_sees_through_optional() {
        let enums = vec![crate::core::ir::EnumDef {
            name: "DataNodeKind".into(),
            ..crate::core::ir::EnumDef::default()
        }];
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "DataNode".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: crate::core::ir::TypeRef::Optional(Box::new(crate::core::ir::TypeRef::Named(
                    "DataNodeKind".into(),
                ))),
                ..crate::core::ir::FieldDef::default()
            }],
            ..crate::core::ir::TypeDef::default()
        }];

        let derived = enum_fields_c_types_from_ir(&type_defs, &enums);

        assert_eq!(derived.get("data_node.kind").map(String::as_str), Some("DataNodeKind"));
    }

    /// A field whose Named type does not match any registered enum (a plain opaque struct,
    /// e.g. `metrics: FileMetrics`) must not be swept in — only genuinely enum-typed fields
    /// get an inferred entry, so opaque-struct fields keep going through the existing
    /// operator-declared `fields_c_types` path unchanged.
    #[test]
    fn enum_fields_c_types_from_ir_ignores_non_enum_named_fields() {
        let enums = vec![crate::core::ir::EnumDef {
            name: "DataNodeKind".into(),
            ..crate::core::ir::EnumDef::default()
        }];
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "ProcessResult".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "metrics".into(),
                ty: crate::core::ir::TypeRef::Named("FileMetrics".into()),
                ..crate::core::ir::FieldDef::default()
            }],
            ..crate::core::ir::TypeDef::default()
        }];

        let derived = enum_fields_c_types_from_ir(&type_defs, &enums);

        assert!(derived.is_empty(), "got: {derived:?}");
    }

    /// End-to-end proof mirroring `enum_fields_from_ir`'s own
    /// `try_emit_enum_accessor_fires_for_a_field_ir_proves_is_an_enum_even_when_fields_enum_omits_it`
    /// test, but starting from a completely EMPTY `fields_c_types` — the exact tslp shape. The
    /// composition `render_test_file` performs (`enum_fields_c_types_from_ir` merged via
    /// `or_insert`, then `enum_fields_from_ir` over the merged map) must be enough for the enum
    /// arm to fire with zero operator configuration.
    #[test]
    fn enum_leaf_accessor_fires_with_zero_operator_config() {
        let enums = vec![crate::core::ir::EnumDef {
            name: "DataNodeKind".into(),
            ..crate::core::ir::EnumDef::default()
        }];
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "DataNode".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: crate::core::ir::TypeRef::Named("DataNodeKind".into()),
                ..crate::core::ir::FieldDef::default()
            }],
            ..crate::core::ir::TypeDef::default()
        }];

        let mut fields_c_types: HashMap<String, String> = HashMap::new();
        for (key, type_name) in enum_fields_c_types_from_ir(&type_defs, &enums) {
            fields_c_types.entry(key).or_insert(type_name);
        }
        let mut fields_enum: HashSet<String> = HashSet::new();
        fields_enum.extend(enum_fields_from_ir(&fields_c_types, &enums));

        let mut out = String::new();
        let mut handles = Vec::new();
        let fired = try_emit_enum_accessor(
            &mut out,
            "ts_pack",
            "TS_PACK",
            "data.kind",
            "kind",
            "data_node",
            "ts_pack_data_node_kind",
            "data_handle",
            "data_kind",
            &fields_c_types,
            &fields_enum,
            &mut handles,
        );

        assert!(fired, "enum accessor must fire with zero fields_c_types config");
        assert!(
            out.contains("ts_pack_data_node_kind_to_string("),
            "must convert via _to_string, not leave a bare handle for strcmp: {out}"
        );
        assert!(!out.contains("char* data_kind = ts_pack_data_node_kind("), "{out}");
    }
}
