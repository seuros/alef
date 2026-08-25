use super::*;

#[test]
fn node_imports_enum_values_referenced_by_typed_inputs() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "processDocument".to_string();
    e2e_config.call.args = vec![crate::e2e::config::ArgMapping {
        name: "input".to_string(),
        field: "input".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some("DocumentInput".to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        id: "process_document".to_string(),
        category: Some("document".to_string()),
        description: "process a document".to_string(),
        input: serde_json::json!({"kind": "uri"}),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let input_type = TypeDef {
        name: "DocumentInput".to_string(),
        fields: vec![crate::core::ir::FieldDef {
            name: "kind".to_string(),
            ty: TypeRef::Named("InputKind".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let enums = [EnumDef {
        name: "InputKind".into(),
        ..Default::default()
    }];

    let output = render_test_file(
        "node",
        "document",
        &[&fixture],
        "",
        "sample-bindings",
        "processDocument",
        &[],
        None,
        None,
        &e2e_config,
        &[input_type],
        &enums,
        &[],
        "",
        &Default::default(),
        &[],
    );

    let binding_import = output
        .lines()
        .find(|line| line.contains("from \"sample-bindings\""))
        .expect("generated test must import its bindings");
    assert!(
        binding_import
            .split([',', '{', '}', ' '])
            .any(|token| token == "InputKind"),
        "Node must import enum classes used as runtime values: {binding_import}"
    );
    assert!(output.contains("kind: InputKind.Uri"), "{output}");
}
