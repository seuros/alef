use std::collections::{HashMap, HashSet};

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

/// A `Named` type resolves to either a struct or an enum definition. Both carry field data
/// that can nest a file input, so `Named` resolution must consult whichever one matches. ~keep
enum NamedDef<'a> {
    Struct(&'a TypeDef),
    Enum(&'a EnumDef),
}

type NamedIndex<'a> = HashMap<&'a str, NamedDef<'a>>;

/// Build a single by-name lookup over both structs and enums, once per top-level call.
/// `Named` resolution then costs an O(1) map lookup instead of a linear scan of the
/// registry at every level of recursion. ~keep
fn build_named_index<'a>(type_defs: &'a [TypeDef], enums: &'a [EnumDef]) -> NamedIndex<'a> {
    let mut index = HashMap::with_capacity(type_defs.len() + enums.len());
    for definition in type_defs {
        index.insert(definition.name.as_str(), NamedDef::Struct(definition));
    }
    for definition in enums {
        index.insert(definition.name.as_str(), NamedDef::Enum(definition));
    }
    index
}

pub(super) fn fixture_uses_test_documents(
    fixture: &Fixture,
    call: &CallConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> bool {
    let index = build_named_index(type_defs, enums);
    fixture.resolved_args(call).iter().any(|argument| {
        if !fixture.docs_files_for_arg(&argument.field).is_empty() || argument.arg_type == "file_path" {
            return true;
        }

        let value = super::resolve_field(&fixture.input, &argument.field);
        if argument.arg_type == "bytes" {
            return value.as_str().is_some_and(is_relative_document_path);
        }
        if argument.arg_type != "json_object" {
            return false;
        }

        argument
            .element_type
            .as_deref()
            .filter(|name| index.contains_key(*name))
            .is_some_and(|name| {
                let element_type = TypeRef::Named(name.to_string());
                let mut visited = HashSet::new();
                if value.is_array() {
                    let vec_type = TypeRef::Vec(Box::new(element_type));
                    typed_value_uses_test_documents(value, &vec_type, &index, &mut visited)
                } else {
                    typed_value_uses_test_documents(value, &element_type, &index, &mut visited)
                }
            })
    })
}

fn typed_value_uses_test_documents(
    value: &serde_json::Value,
    ty: &TypeRef,
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    match ty {
        TypeRef::Bytes => value.as_str().is_some_and(is_relative_document_path),
        TypeRef::Optional(inner) => typed_value_uses_test_documents(value, inner, index, visited),
        TypeRef::Vec(inner) => value.as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| typed_value_uses_test_documents(value, inner, index, visited))
        }),
        TypeRef::Map(_, value_type) => value.as_object().is_some_and(|values| {
            values
                .values()
                .any(|value| typed_value_uses_test_documents(value, value_type, index, visited))
        }),
        TypeRef::Named(name) => resolve_named_uses_test_documents(value, name, index, visited),
        _ => false,
    }
}

/// Resolve a `Named` type against the combined struct/enum index, guarding against cycles.
///
/// A `#[serde(flatten)]` field recurses against the SAME JSON value (see
/// `fields_use_test_documents`) rather than a smaller sub-value, so — unlike the rest of this
/// traversal — recursion here is no longer naturally bounded by the shrinking size of the JSON
/// value. A self-referential TypeDef/EnumDef reached only through flattened fields would recurse
/// forever on the same value. `visited` tracks names already being resolved along the current
/// path and is removed on the way back out, so sibling branches may still revisit the same named
/// type — only a true cycle on the active path is rejected. ~keep
fn resolve_named_uses_test_documents(
    value: &serde_json::Value,
    name: &str,
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    if !visited.insert(name.to_string()) {
        return false;
    }
    let found = match index.get(name) {
        Some(NamedDef::Struct(definition)) => struct_value_uses_test_documents(value, definition, index, visited),
        Some(NamedDef::Enum(definition)) => enum_value_uses_test_documents(value, definition, index, visited),
        None => false,
    };
    visited.remove(name);
    found
}

fn struct_value_uses_test_documents(
    value: &serde_json::Value,
    definition: &TypeDef,
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    fields_use_test_documents(
        value,
        &definition.fields,
        definition.serde_rename_all.as_deref(),
        index,
        visited,
    )
}

/// Walk an object's fields against a JSON object value, shared by struct bodies and
/// struct-shaped enum variants (both are "named fields against an object" in the same way). ~keep
fn fields_use_test_documents(
    value: &serde_json::Value,
    fields: &[FieldDef],
    rename_all: Option<&str>,
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    fields.iter().any(|field| {
        if field.serde_flatten {
            // Flattened fields have no wire key of their own -- their sub-fields appear
            // inline in the SAME parent object, so recurse against `value`, not a nested
            // sub-value. ~keep
            return typed_value_uses_test_documents(value, &field.ty, index, visited);
        }
        let wire_name = crate::codegen::naming::wire_field_name(&field.name, field.serde_rename.as_deref(), rename_all);
        object
            .get(&field.name)
            .or_else(|| object.get(&wire_name))
            .is_some_and(|value| typed_value_uses_test_documents(value, &field.ty, index, visited))
    })
}

fn enum_value_uses_test_documents(
    value: &serde_json::Value,
    definition: &EnumDef,
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    definition
        .variants
        .iter()
        .any(|variant| variant_uses_test_documents(value, definition, variant, index, visited))
}

fn variant_uses_test_documents(
    value: &serde_json::Value,
    definition: &EnumDef,
    variant: &EnumVariant,
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    let Some(candidate) = variant_payload(value, definition, variant) else {
        return false;
    };
    if variant.is_tuple {
        return tuple_variant_uses_test_documents(candidate, &variant.fields, index, visited);
    }
    // `definition.serde_rename_all` cases the enum's VARIANT names (used above, in
    // `variant_payload`) -- a different serde namespace from how this variant's own payload
    // FIELDS are cased. `EnumVariant` carries no per-variant field-casing rule in the IR, so
    // there is no correct value to pass here; borrowing the enum's rule produced false matches
    // whenever a field happened to collide with that unrelated casing. Pass `None` so only the
    // raw field name and each field's own explicit `serde_rename` are tried (both still handled
    // by `fields_use_test_documents`'s `.or_else` fallback). ~keep
    fields_use_test_documents(candidate, &variant.fields, None, index, visited)
}

/// Locate the sub-value that carries a variant's payload, per the enum's serde tagging style. ~keep
fn variant_payload<'a>(
    value: &'a serde_json::Value,
    definition: &EnumDef,
    variant: &EnumVariant,
) -> Option<&'a serde_json::Value> {
    if definition.serde_untagged {
        return Some(value);
    }
    let Some(tag_key) = &definition.serde_tag else {
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            definition.serde_rename_all.as_deref(),
        );
        return value.get(&wire_name);
    };
    // Internally/adjacently tagged: the tag key's value must actually name THIS variant before
    // its fields are walked. Two variants can otherwise share a field name and have the wrong
    // one's type "accidentally" match the selected variant's real payload. ~keep
    if !tag_matches_variant(value, tag_key, definition, variant) {
        return None;
    }
    match &definition.serde_content {
        Some(content_key) => value.get(content_key),
        // Internally tagged: variant fields sit inline in the same object as the tag key. ~keep
        None => Some(value),
    }
}

/// True when `value`'s tag key names exactly this variant, via the SAME wire-name derivation
/// `wire_variant_value` computes above for externally tagged enums -- routing through the
/// central helper here, rather than re-deriving casing locally, is what keeps a comparison
/// mistake from silently missing a real variant match, and thus a real file input. ~keep
fn tag_matches_variant(value: &serde_json::Value, tag_key: &str, definition: &EnumDef, variant: &EnumVariant) -> bool {
    let wire_name = crate::codegen::naming::wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        definition.serde_rename_all.as_deref(),
    );
    value.get(tag_key).and_then(serde_json::Value::as_str) == Some(wire_name.as_str())
}

fn tuple_variant_uses_test_documents(
    candidate: &serde_json::Value,
    fields: &[FieldDef],
    index: &NamedIndex<'_>,
    visited: &mut HashSet<String>,
) -> bool {
    if let [only] = fields {
        return typed_value_uses_test_documents(candidate, &only.ty, index, visited);
    }
    candidate.as_array().is_some_and(|values| {
        fields
            .iter()
            .zip(values.iter())
            .any(|(field, value)| typed_value_uses_test_documents(value, &field.ty, index, visited))
    })
}

fn is_relative_document_path(value: &str) -> bool {
    if value.starts_with('<') || value.starts_with('{') || value.starts_with('[') || value.contains(' ') {
        return false;
    }
    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_alphanumeric() && first != '_' {
        return false;
    }
    value
        .find('/')
        .map(|slash| &value[slash + 1..])
        .is_some_and(|suffix| !suffix.is_empty() && suffix.contains('.'))
}

#[cfg(test)]
mod cycle_guard_tests;
#[cfg(test)]
mod tag_and_shape_tests;
#[cfg(test)]
mod tag_value_discrimination_tests;
#[cfg(test)]
mod variant_field_rename_all_tests;

#[cfg(test)]
mod tests {
    use crate::core::config::e2e::{ArgMapping, CallConfig};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn object_arg() -> ArgMapping {
        ArgMapping {
            name: "request".into(),
            field: "input".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: true,
            element_type: Some("SampleRequest".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn request_type() -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn nested_bytes_file_path_requires_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"content": "documents/sample.bin"}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_type()],
            &[]
        ));
    }

    #[test]
    fn nested_inline_bytes_do_not_require_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"content": "inline text"}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_type()],
            &[]
        ));
    }

    #[test]
    fn batch_nested_bytes_file_path_requires_test_document_working_directory() {
        let mut argument = object_arg();
        argument.field = "input.requests".into();
        let fixture = Fixture {
            input: serde_json::json!({
                "requests": [
                    {"content": "inline text"},
                    {"content": "documents/sample.bin"}
                ]
            }),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![argument],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_type()],
            &[]
        ));
    }

    /// Externally tagged (serde default) enum with one struct-shaped variant carrying bytes. ~keep
    fn event_enum() -> EnumDef {
        EnumDef {
            name: "SampleEvent".into(),
            variants: vec![EnumVariant {
                name: "Uploaded".into(),
                fields: vec![FieldDef {
                    name: "file".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn request_with_event_type() -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "event".into(),
                ty: TypeRef::Named("SampleEvent".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn enum_variant_payload_bytes_file_path_requires_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"event": {"Uploaded": {"file": "documents/sample.bin"}}}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_event_type()],
            &[event_enum()]
        ));
    }

    #[test]
    fn enum_variant_payload_inline_bytes_do_not_require_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"event": {"Uploaded": {"file": "inline text"}}}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_event_type()],
            &[event_enum()]
        ));
    }

    #[test]
    fn enum_variant_mismatched_tag_key_does_not_require_test_document_working_directory() {
        // Control: the JSON payload names a variant that does not exist on the enum, so
        // no variant's wire key resolves and no field should be considered reachable. ~keep
        let fixture = Fixture {
            input: serde_json::json!({"event": {"SomethingElse": {"file": "documents/sample.bin"}}}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_event_type()],
            &[event_enum()]
        ));
    }

    fn flattened_details_type() -> TypeDef {
        TypeDef {
            name: "SampleDetails".into(),
            fields: vec![FieldDef {
                name: "attachment".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn request_with_flattened_details() -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "details".into(),
                ty: TypeRef::Named("SampleDetails".into()),
                serde_flatten: true,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn flattened_named_field_bytes_file_path_requires_test_document_working_directory() {
        // `details` is flattened, so its `attachment` field appears at the SAME level as
        // the parent object, not nested under a `"details"` key. ~keep
        let fixture = Fixture {
            input: serde_json::json!({"attachment": "documents/sample.bin"}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_flattened_details(), flattened_details_type()],
            &[]
        ));
    }
}
