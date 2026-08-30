use super::*;

/// The WASM setter-builder uses the binding field name, not its serialized wire name. ~keep
#[test]
fn wasm_field_keyed_by_a_wire_name_that_diverges_from_its_js_name_resolves_the_js_name() {
    let type_defs = [TypeDef {
        name: "ExampleTool".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "tool_type".into(),
            ty: TypeRef::String,
            serde_rename: Some("type".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let expression = ts_builder_expression(
        serde_json::json!({"type": "function"}).as_object().expect("object"),
        "WasmExampleTool",
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        "Wasm",
        &[],
        &mut Default::default(),
    );

    assert!(expression.contains("_u0.toolType = \"function\""), "{expression}");
}
