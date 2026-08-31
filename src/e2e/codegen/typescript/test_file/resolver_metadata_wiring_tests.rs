use super::render_test_case;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashMap;

fn render_fixture(assertion: Assertion, fields: Vec<FieldDef>, enums: Vec<EnumDef>) -> String {
    let fixture = Fixture {
        id: "metadata_wiring".to_string(),
        description: "metadata wiring".to_string(),
        assertions: vec![assertion],
        ..Fixture::default()
    };
    let call = CallConfig {
        function: "make_report".to_string(),
        result_fields: fields.iter().map(|field| field.name.clone()).collect(),
        ..CallConfig::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..E2eConfig::default()
    };
    let type_defs = vec![TypeDef {
        name: "Report".to_string(),
        fields,
        ..TypeDef::default()
    }];
    let functions = vec![FunctionDef {
        name: "make_report".to_string(),
        return_type: TypeRef::Named("Report".to_string()),
        ..FunctionDef::default()
    }];
    let result_enum_fields = HashMap::from([("kind".to_string(), "WasmPayload".to_string())]);
    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        "wasm",
        &HashMap::new(),
        &HashMap::new(),
        &result_enum_fields,
        &type_defs,
        &enums,
        &functions,
        "Wasm",
        &ResolvedCrateConfig::default(),
        &mut Default::default(),
        &[],
    );
    out
}

#[test]
fn real_test_case_routes_internal_wasm_enums_through_their_tag() {
    let payload = EnumDef {
        name: "Payload".to_string(),
        serde_tag: Some("kind-tag".to_string()),
        variants: vec![EnumVariant {
            name: "Custom".to_string(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };
    let out = render_fixture(
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::json!("Custom")),
            ..Assertion::default()
        },
        vec![FieldDef {
            name: "kind".to_string(),
            ty: TypeRef::Named("Payload".to_string()),
            ..FieldDef::default()
        }],
        vec![payload],
    );
    assert!(
        out.contains("expect(result.kind?.[\"kind-tag\"]).toBe(\"Custom\");"),
        "{out}"
    );
}

#[test]
fn real_test_case_keeps_numeric_collections_off_the_text_surface() {
    let out = render_fixture(
        Assertion {
            assertion_type: "contains".to_string(),
            field: Some("codes".to_string()),
            value: Some(serde_json::json!("42")),
            ..Assertion::default()
        },
        vec![FieldDef {
            name: "codes".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
            ..FieldDef::default()
        }],
        vec![],
    );
    assert!(out.contains("String(item) === \"42\""), "{out}");
    assert!(!out.contains("_alefE2eItemTexts(item)"), "{out}");
}
