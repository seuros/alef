use std::collections::HashSet;

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{DefaultValue, EnumDef, EnumVariant, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::assertion_field_shape::resolve_assertion_field_shape;
use super::test_function::{GoTestFunctionContext, render_test_function};

fn assert_rendered_go_compiles(rendered: &str, sample_source: &str) {
    let Ok(go) = which::which("go") else {
        eprintln!("Go compiler unavailable; skipping rendered assertion compile fixture");
        return;
    };
    let directory = tempfile::tempdir().expect("create generated Go fixture");
    std::fs::write(
        directory.path().join("go.mod"),
        "module example.com/sample\n\ngo 1.24\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("sample.go"), sample_source).unwrap();
    let mut imports = vec!["\"testing\"", "sample \"example.com/sample\""];
    if rendered.contains("strings.") {
        imports.push("\"strings\"");
    }
    if rendered.contains("jsonString(") {
        imports.push("\"encoding/json\"");
    }
    let assertion_stub = if rendered.contains("assert.") {
        "type assertions struct{}\nvar assert assertions\nfunc (assertions) NotNil(*testing.T, any, ...string) {}\nfunc (assertions) GreaterOrEqual(*testing.T, any, any, ...string) {}\nfunc (assertions) LessOrEqual(*testing.T, any, any, ...string) {}\nfunc (assertions) Equal(*testing.T, any, any, ...string) {}\n"
    } else {
        ""
    };
    let json_stub = if rendered.contains("jsonString(") {
        "func jsonString(value any) string { data, _ := json.Marshal(value); return string(data) }\n"
    } else {
        ""
    };
    let source = format!(
        "package sample_test\nimport ({})\n{assertion_stub}{json_stub}\n{rendered}",
        imports.join("\n")
    );
    std::fs::write(directory.path().join("shape_test.go"), source).unwrap();
    let output = std::process::Command::new(go)
        .args(["test", "./..."])
        .current_dir(directory.path())
        .output()
        .expect("run Go compiler");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}\n{rendered}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn render_field_assertion(
    field: FieldDef,
    assertion_field: &str,
    enums: &[EnumDef],
    configured_optional: bool,
    assertion_type: &str,
    value: Option<serde_json::Value>,
) -> String {
    let mut optional = HashSet::new();
    if configured_optional {
        optional.insert(field.name.clone());
    }
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        fields_optional: optional,
        ..Default::default()
    };
    let uses_values = matches!(assertion_type, "contains_all" | "contains_any" | "not_contains");
    let values = uses_values.then(|| vec![value.clone().expect("string family value")]);
    let fixture = Fixture {
        id: "field_shape".into(),
        description: "field shape".into(),
        assertions: vec![Assertion {
            assertion_type: assertion_type.into(),
            field: Some(assertion_field.into()),
            value: (!uses_values).then_some(value).flatten(),
            values,
            ..Default::default()
        }],
        ..Default::default()
    };
    render_fixture(config, fixture, field, enums)
}

fn render_fixture(config: E2eConfig, fixture: Fixture, field: FieldDef, enums: &[EnumDef]) -> String {
    let type_defs = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![field],
        ..Default::default()
    }];
    let functions = vec![FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    let mut output = String::new();
    render_test_function(
        &mut output,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample",
            e2e_config: &config,
            adapters: &[],
            data_enum_names: &HashSet::new(),
            config: &Default::default(),
            type_defs: &type_defs,
            enums,
            errors: &[],
            functions: &functions,
        },
    );
    output
}

#[test]
fn optional_data_interface_field_is_nullable_but_not_dereferenced() {
    let choice = EnumDef {
        name: "Choice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            fields: vec![FieldDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let output = render_field_assertion(
        FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            optional: true,
            ..Default::default()
        },
        "choice",
        &[choice],
        true,
        "is_true",
        None,
    );

    assert!(
        !output.contains("*result.Choice"),
        "sealed interfaces are not pointers:\n{output}"
    );
    assert_rendered_go_compiles(
        &output,
        "package sample\ntype Choice interface{}\ntype Envelope struct { Choice Choice }\nfunc Inspect() (*Envelope, error) { return &Envelope{Choice: \"value\"}, nil }\n",
    );
}

#[test]
fn required_unresolved_named_field_uses_raw_message_pointer_shape() {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "payload".into(),
            ty: TypeRef::Named("ForeignPayload".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts_with_enums(&types, &[], "go"),
        Some("Envelope".into()),
    );
    assert_eq!(resolver.target_field_is_pointer("payload"), Some(true));

    let output = render_field_assertion(
        FieldDef {
            name: "payload".into(),
            ty: TypeRef::Named("ForeignPayload".into()),
            ..Default::default()
        },
        "payload",
        &[],
        false,
        "contains",
        Some(serde_json::json!("sample")),
    );

    assert!(
        output.contains("*result.Payload"),
        "unresolved named fields are pointers:\n{output}"
    );
    assert_rendered_go_compiles(
        &output,
        "package sample\nimport \"encoding/json\"\ntype Envelope struct { Payload *json.RawMessage }\nfunc Inspect() (*Envelope, error) { raw := json.RawMessage(`{\"value\":\"sample\"}`); return &Envelope{Payload: &raw}, nil }\n",
    );
}

#[test]
fn optional_vec_assertions_follow_go_slice_shape_over_global_optionality() {
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        fields_optional: HashSet::from(["items".into()]),
        fields_array: HashSet::from(["items".into()]),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "optional_vec_shape".into(),
        assertions: vec![Assertion {
            assertion_type: "min_length".into(),
            field: Some("items".into()),
            value: Some(serde_json::json!(1)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let field = FieldDef {
        name: "items".into(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        optional: true,
        ..Default::default()
    };
    let output = render_fixture(config, fixture, field, &[]);

    assert!(output.contains("len(result.Items)"), "expected slice length:\n{output}");
    assert!(
        !output.contains("len(*result.Items)"),
        "must not dereference slice:\n{output}"
    );
}

#[test]
fn optional_local_has_plain_value_shape() {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "title".into(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts_with_enums(&types, &[], "go"),
        Some("Envelope".into()),
    );
    let assertion = Assertion {
        assertion_type: "equals".into(),
        field: Some("title".into()),
        value: Some(serde_json::json!("sample")),
        ..Default::default()
    };
    let locals = std::collections::HashMap::from([("title".into(), "title".into())]);
    let shape = resolve_assertion_field_shape(&assertion, &resolver, &locals);

    assert!(!shape.is_optional);
    assert!(!shape.is_pointer);
    assert!(!shape.is_nullable);
}

#[test]
fn required_default_string_count_dereferences_authoritative_pointer() {
    let output = render_field_assertion(
        FieldDef {
            name: "label".into(),
            ty: TypeRef::String,
            default: Some("default_label".into()),
            typed_default: Some(DefaultValue::StringLiteral("default".into())),
            ..Default::default()
        },
        "label",
        &[],
        false,
        "count_min",
        Some(serde_json::json!(1)),
    );

    assert!(output.contains("len(*result.Label)"), "{output}");
    assert_rendered_go_compiles(
        &output,
        "package sample\ntype Envelope struct { Label *string }\nfunc Inspect() (*Envelope, error) { value := \"sample\"; return &Envelope{Label: &value}, nil }\n",
    );
}

#[test]
fn required_default_number_comparison_dereferences_authoritative_pointer() {
    let output = render_field_assertion(
        FieldDef {
            name: "limit".into(),
            ty: TypeRef::Primitive(PrimitiveType::I64),
            default: Some("default_limit".into()),
            typed_default: Some(DefaultValue::IntLiteral(5)),
            ..Default::default()
        },
        "limit",
        &[],
        false,
        "greater_than",
        Some(serde_json::json!(1)),
    );

    assert!(output.contains("*result.Limit < 2"), "{output}");
    assert_rendered_go_compiles(
        &output,
        "package sample\ntype Envelope struct { Limit *int64 }\nfunc Inspect() (*Envelope, error) { value := int64(5); return &Envelope{Limit: &value}, nil }\n",
    );
}

fn assert_pointer_pseudo_field_compiles(suffix: &str, assertion_type: &str) {
    let field_path = format!("label.{suffix}");
    let expected = match assertion_type {
        "greater_than" => 0,
        "less_than_or_equal" | "max_length" => 10,
        "count_equals" => 6,
        _ => 1,
    };
    let output = render_field_assertion(
        FieldDef {
            name: "label".into(),
            ty: TypeRef::String,
            default: Some("default_label".into()),
            typed_default: Some(DefaultValue::StringLiteral("default".into())),
            ..Default::default()
        },
        &field_path,
        &[],
        false,
        assertion_type,
        Some(serde_json::json!(expected)),
    );
    assert!(!output.contains("len(*result.Label) != nil"), "{output}");
    assert!(!output.contains("len(len(*result.Label))"), "{output}");
    assert_rendered_go_compiles(
        &output,
        "package sample\ntype Envelope struct { Label *string }\nfunc Inspect() (*Envelope, error) { value := \"sample\"; return &Envelope{Label: &value}, nil }\n",
    );
}

#[test]
fn pointer_length_and_count_pseudo_fields_compile_as_scalars() {
    for suffix in ["length", "count"] {
        for assertion_type in [
            "greater_than",
            "less_than_or_equal",
            "count_min",
            "count_equals",
            "min_length",
            "max_length",
        ] {
            assert_pointer_pseudo_field_compiles(suffix, assertion_type);
        }
    }
}

fn assert_data_interface_string_family_compiles(assertion_type: &str, expected: &str) {
    let choice = EnumDef {
        name: "Choice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            fields: vec![FieldDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let output = render_field_assertion(
        FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            ..Default::default()
        },
        "choice",
        &[choice],
        false,
        assertion_type,
        Some(serde_json::json!(expected)),
    );
    assert!(output.contains("jsonString(result.Choice)"), "{output}");
    assert_rendered_go_compiles(
        &output,
        "package sample\ntype Choice interface{}\ntype Envelope struct { Choice Choice }\nfunc Inspect() (*Envelope, error) { return &Envelope{Choice: \"value\"}, nil }\n",
    );
}

#[test]
fn data_interface_string_assertion_families_compile_with_wire_json() {
    for (assertion_type, expected) in [
        ("equals", "value"),
        ("contains", "value"),
        ("contains_all", "value"),
        ("not_contains", "absent"),
        ("contains_any", "value"),
    ] {
        assert_data_interface_string_family_compiles(assertion_type, expected);
    }
}

#[test]
fn go_result_shapes_follow_emitted_type_partitions() {
    let (types, enums, excluded) = partitioned_type_fixture();
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::go_ir_result_field_facts(&types, &enums, &excluded),
        Some("Envelope".into()),
    );

    for field in ["excluded", "opaque", "visitor", "enum_value"] {
        assert_eq!(resolver.target_field_is_pointer(field), Some(true), "{field}");
    }
}

fn partitioned_type_fixture() -> (Vec<TypeDef>, Vec<EnumDef>, HashSet<String>) {
    let named_field = |name: &str, target: &str| FieldDef {
        name: name.into(),
        ty: TypeRef::Named(target.into()),
        ..Default::default()
    };
    let types = vec![
        TypeDef {
            name: "Envelope".into(),
            fields: vec![
                named_field("excluded", "Excluded"),
                named_field("opaque", "Opaque"),
                named_field("visitor", "VisitorContext"),
                named_field("enum_value", "HiddenChoice"),
            ],
            ..Default::default()
        },
        TypeDef {
            name: "Excluded".into(),
            ..Default::default()
        },
        TypeDef {
            name: "Opaque".into(),
            is_opaque: true,
            ..Default::default()
        },
        TypeDef {
            name: "VisitorContext".into(),
            ..Default::default()
        },
    ];
    let enums = vec![EnumDef {
        name: "HiddenChoice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let excluded = HashSet::from(["Excluded".into(), "VisitorContext".into(), "HiddenChoice".into()]);
    (types, enums, excluded)
}
