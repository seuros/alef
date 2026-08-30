use crate::core::config::e2e::CallConfig;
use crate::core::ir::{TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

pub(super) fn fixture_uses_test_documents(fixture: &Fixture, call: &CallConfig, type_defs: &[TypeDef]) -> bool {
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
            .and_then(|name| type_defs.iter().find(|definition| definition.name == name))
            .is_some_and(|definition| {
                let element_type = TypeRef::Named(definition.name.clone());
                if value.is_array() {
                    typed_value_uses_test_documents(value, &TypeRef::Vec(Box::new(element_type)), type_defs)
                } else {
                    typed_value_uses_test_documents(value, &element_type, type_defs)
                }
            })
    })
}

fn typed_value_uses_test_documents(value: &serde_json::Value, ty: &TypeRef, type_defs: &[TypeDef]) -> bool {
    match ty {
        TypeRef::Bytes => value.as_str().is_some_and(is_relative_document_path),
        TypeRef::Optional(inner) => typed_value_uses_test_documents(value, inner, type_defs),
        TypeRef::Vec(inner) => value.as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| typed_value_uses_test_documents(value, inner, type_defs))
        }),
        TypeRef::Map(_, value_type) => value.as_object().is_some_and(|values| {
            values
                .values()
                .any(|value| typed_value_uses_test_documents(value, value_type, type_defs))
        }),
        TypeRef::Named(name) => type_defs
            .iter()
            .find(|definition| definition.name == *name)
            .is_some_and(|definition| struct_value_uses_test_documents(value, definition, type_defs)),
        _ => false,
    }
}

fn struct_value_uses_test_documents(value: &serde_json::Value, definition: &TypeDef, type_defs: &[TypeDef]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    definition.fields.iter().any(|field| {
        let wire_name = crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            definition.serde_rename_all.as_deref(),
        );
        object
            .get(&field.name)
            .or_else(|| object.get(&wire_name))
            .is_some_and(|value| typed_value_uses_test_documents(value, &field.ty, type_defs))
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
mod tests {
    use crate::core::config::e2e::{ArgMapping, CallConfig};
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
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

        assert!(super::fixture_uses_test_documents(&fixture, &call, &[request_type()]));
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

        assert!(!super::fixture_uses_test_documents(&fixture, &call, &[request_type()]));
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

        assert!(super::fixture_uses_test_documents(&fixture, &call, &[request_type()]));
    }
}
