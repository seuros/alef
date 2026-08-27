//! A derived snippet accessor must be judged on its WHOLE path, not on its first segment.
//!
//! ~keep [`super::anchored_result_facts_tests`] anchored the availability oracle at the call's own
//! result type and closed the root-segment hole. The same hole stayed open one step in: the
//! anchored answer came from `ir_result_fields::root_declares_first_segment`, so any path whose
//! FIRST segment was a real field was waved through whatever it named afterwards. A name that only
//! ever appears in `alef.toml` — a `result_fields` entry, a `fields_method_calls` path — hung off a
//! genuine field therefore became a member access on a type that declares no such member, and one
//! consumer shipped 28 non-compiling snippets on it (`Property 'documentStructure' does not exist
//! on type 'ExtractedDocument'`).
//!
//! The asymmetry the sibling module asserts is preserved here at depth, and so is its
//! conservatism: a path that walks OUT of the struct graph the IR carries (into a map value, a
//! `serde_json::Value`, a primitive, a foreign type) is still unjudgeable, and must keep deriving
//! its accessor rather than be rejected for being unrecognized.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

/// A docs-tagged fixture with neither `shows` nor `presentation`, so every operation is derived.
fn docs_fixture(assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": assertions,
        "docs": {"topic": "smoke", "stem": "sample_fixture"}
    }))
    .expect("fixture must parse")
}

/// The consumer shape: a name that exists ONLY in `alef.toml`. Listing it in `result_fields` and
/// `fields_method_calls` is what makes `is_valid_for_result` keep saying yes, which is the whole
/// reason the derived path needs its own, stricter oracle. ~keep
const CONFIG_ONLY_LEAF: &str = "document_structure";

fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        result_fields: ["document".to_string(), CONFIG_ONLY_LEAF.to_string()]
            .into_iter()
            .collect(),
        fields_method_calls: [format!("document.{CONFIG_ONLY_LEAF}")].into_iter().collect(),
        ..E2eConfig::default()
    }
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

fn convert_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

/// `convert` -> `ConversionResult { document: ExtractedDocument }`, and `ExtractedDocument`
/// declares `nodes` and nothing else. `payload` is deliberately a plain string: it is the segment
/// the IR can confirm exists but cannot look inside.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![field(
                "document",
                TypeRef::Named("ExtractedDocument".to_string()),
                false,
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ExtractedDocument".to_string(),
            fields: vec![
                field("nodes", TypeRef::String, false),
                field("payload", TypeRef::Json, false),
            ],
            ..TypeDef::default()
        },
    ]
}

fn shown_expressions(field_path: &str) -> Vec<String> {
    resolve(
        &docs_fixture(serde_json::json!([{"type": "equals", "field": field_path, "value": "Hello"}])),
        &config(),
        "python",
        &type_defs(),
        &[],
        &convert_returning("ConversionResult"),
    )
    .into_iter()
    .map(|operation| operation.expression)
    .collect()
}

/// The defect: `document` is real, `document_structure` is a config key, and judging only the
/// first segment published `result.document.document_structure`. ~keep
#[test]
fn a_config_only_name_hung_off_a_declared_field_derives_no_accessor() {
    assert_eq!(
        shown_expressions(&format!("document.{CONFIG_ONLY_LEAF}")),
        Vec::<String>::new(),
        "`{CONFIG_ONLY_LEAF}` is declared by alef.toml, not by ExtractedDocument"
    );
}

/// The control that keeps the fix from being "reject every nested path": a leaf the nested type
/// really declares must still be shown, at the same depth, through the same resolver. ~keep
#[test]
fn a_nested_field_the_result_type_declares_still_derives_its_accessor() {
    assert_eq!(shown_expressions("document.nodes"), vec!["result.document.nodes"]);
}

/// The conservatism `root_declares_path` inherits: `payload` is a real field whose type the IR
/// carries no fields for, so nothing past it can be judged and the accessor must still derive.
/// Rejecting here would discard every real map/JSON/foreign-type traversal in the suite. ~keep
#[test]
fn a_path_walking_out_of_the_ir_struct_graph_still_derives_its_accessor() {
    assert_eq!(
        shown_expressions("document.payload.anything"),
        vec!["result.document.payload.anything"]
    );
}

/// The same asymmetry [`super::anchored_result_facts_tests`] pins at the root, asserted one level
/// down on ONE resolver and ONE path: a hand-authored assertion on the config-declared path must
/// still render, while the inferred accessor for it must be refused. ~keep
#[test]
fn the_availability_oracles_disagree_on_purpose_for_a_nested_name_the_result_type_lacks() {
    let e2e_config = config();
    let resolver = build_resolver(
        &e2e_config,
        &e2e_config.call,
        "python",
        &type_defs(),
        &[],
        &convert_returning("ConversionResult"),
    );

    let absent = format!("document.{CONFIG_ONLY_LEAF}");
    assert!(
        resolver.is_valid_for_result(&absent),
        "a hand-authored assertion on `{absent}` must still render"
    );
    assert_eq!(
        resolver.result_field_oracle_knows(&absent),
        Some(false),
        "an inferred accessor for `{absent}` must be refused"
    );

    assert!(resolver.is_valid_for_result("document.nodes"));
    assert_eq!(resolver.result_field_oracle_knows("document.nodes"), Some(true));
    // The anchored walk abstains at `payload` and the flat, name-keyed answer takes over — which
    // is exactly the pre-anchoring verdict, and the reason the accessor above still derives. ~keep
    assert_eq!(
        resolver.result_field_oracle_knows("document.payload.anything"),
        Some(true)
    );
}

/// A refused path the consumer wrote down in `alef.toml` is drift they can fix, so the warning
/// has to name the key that carries it — and a path nobody declared must stay silent, or the
/// diagnostic drowns in every assertion grouping and streaming pseudo-field in the suite. ~keep
#[test]
fn only_a_consumer_declared_path_names_the_config_key_that_declares_it() {
    let e2e_config = config();
    let resolver = build_resolver(
        &e2e_config,
        &e2e_config.call,
        "python",
        &type_defs(),
        &[],
        &convert_returning("ConversionResult"),
    );

    assert_eq!(
        resolver.declaring_config_key(&format!("document.{CONFIG_ONLY_LEAF}")),
        Some("fields_method_calls")
    );
    assert_eq!(resolver.declaring_config_key("document.nodes"), None);
    assert_eq!(resolver.declaring_config_key("rate_limit.min_duration_ms"), None);
}
