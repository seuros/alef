//! Regression coverage for the Swift *snippet* generator's treatment of a JSON-bridged leaf.
//!
//! swift-bridge collapses a JSON-bridged field to one `RustString`, which has no elements and no
//! subscript. The e2e generator already refuses every step past such a leaf and writes a skip
//! comment saying so; the snippet generator asked nothing and emitted the subscript anyway, so a
//! documentation snippet could not compile while the e2e file generated beside it from the same IR
//! declared that exact step impossible.
//!
//! These tests drive both real entry points — `render_test_method` and `snippet::render_with_ir` —
//! against one IR and assert the two verdicts match, so a fix that teaches only one generator the
//! rule cannot pass.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// The e2e generator's wording for "this step is unspellable in Swift".
const JSON_BRIDGE_SKIP: &str = "swift-bridge JSON-bridges it to RustString";

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// IR whose getter shapes are the ones the Swift backend actually emits: `labels` and `headings`
/// JSON-bridge to a `RustString`, `sections` stays a countable `RustVec`.
fn bridged_ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "SectionInfo".to_string(),
            fields: vec![
                field("level", TypeRef::Primitive(PrimitiveType::U32), false),
                field("text", TypeRef::String, false),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "PageMetadata".to_string(),
            fields: vec![
                field("title", TypeRef::String, false),
                // `HashMap<String, String>` -> `fn labels(&self) -> String` (the whole map as JSON).
                field(
                    "labels",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
                // `Option<Vec<SectionInfo>>` -> `fn headings(&self) -> String`.
                field(
                    "headings",
                    TypeRef::Vec(Box::new(TypeRef::Named("SectionInfo".to_string()))),
                    true,
                ),
                // `Vec<SectionInfo>` -> `fn sections(&self) -> RustVec<SectionInfo>`: the negative
                // control. An indiscriminate refusal would take this one down with the others.
                field(
                    "sections",
                    TypeRef::Vec(Box::new(TypeRef::Named("SectionInfo".to_string()))),
                    false,
                ),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![field("metadata", TypeRef::Named("PageMetadata".to_string()), false)],
            has_serde: true,
            ..TypeDef::default()
        },
    ];
    let functions = vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("ProcessResult".to_string()),
        ..FunctionDef::default()
    }];
    (type_defs, functions)
}

fn e2e_config() -> (E2eConfig, CallConfig) {
    let call_config = CallConfig {
        function: "process".to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("process".to_string(), call_config.clone());
    e2e_config.result_fields = ["metadata".to_string()].into_iter().collect();
    (e2e_config, call_config)
}

fn fixture_showing(path: &str) -> Fixture {
    fixture_with_operation(path, serde_json::json!({"op": "show", "path": path, "display": true}))
}

fn fixture_iterating(path: &str, item: &str, fields: &[&str]) -> Fixture {
    fixture_with_operation(
        path,
        serde_json::json!({"op": "iterate", "path": path, "item": item, "fields": fields}),
    )
}

/// One fixture drives both generators: `docs.presentation` is what the snippet renders, and the
/// identically-pathed `assertions` entry is what the e2e test method renders.
fn fixture_with_operation(path: &str, operation: serde_json::Value) -> Fixture {
    Fixture {
        id: "bridged_leaf".to_string(),
        description: "Bridged leaf".to_string(),
        call: Some("process".to_string()),
        docs: serde_json::from_value(serde_json::json!({
            "topic": "guides",
            "presentation": {"operations": [operation]}
        }))
        .expect("docs must parse"),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some(path.to_string()),
            value: Some(serde_json::json!("Example")),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn render_e2e(fixture: &Fixture) -> String {
    let (type_defs, functions) = bridged_ir();
    let (e2e, call_config) = e2e_config();
    let map = super::values::build_swift_first_class_map(&type_defs, &[], &e2e, &call_config);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        fixture,
        &e2e,
        "process",
        "result",
        &[],
        false,
        None,
        &map,
        "Sample",
        &config,
        &type_defs,
        &[],
        &functions,
        &[],
    );
    out
}

fn render_snippet(fixture: &Fixture) -> String {
    let (type_defs, functions) = bridged_ir();
    let (e2e, _) = e2e_config();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    super::snippet::render_with_ir(fixture, &e2e, &config, &type_defs, &[], &functions).expect("snippet renders")
}

/// The reported shape: a string-keyed subscript on a field the binding collapsed to one
/// `RustString`. `labels()["theme"]` is not spellable, and the e2e file rendered from the same IR
/// says so on the line the snippet contradicted.
#[test]
fn should_clamp_a_map_subscript_to_the_bridged_leaf_the_e2e_generator_refuses() {
    let fixture = fixture_showing("metadata.labels[\"theme\"]");
    let e2e = render_e2e(&fixture);
    let snippet = render_snippet(&fixture);

    assert!(
        e2e.contains(JSON_BRIDGE_SKIP),
        "premise: the e2e generator must refuse this step, got:\n{e2e}"
    );
    assert!(
        !snippet.contains("labels()["),
        "the snippet must not subscript a RustString leaf, got:\n{snippet}"
    );
    assert!(
        snippet.contains("print(result.metadata().labels())"),
        "the snippet must fall back to the readable bridged leaf, got:\n{snippet}"
    );
}

/// The same impossibility spelled as an index plus a member read.
#[test]
fn should_clamp_an_indexed_step_into_a_bridged_leaf() {
    let fixture = fixture_showing("metadata.headings[0].text");
    let e2e = render_e2e(&fixture);
    let snippet = render_snippet(&fixture);

    assert!(
        e2e.contains(JSON_BRIDGE_SKIP),
        "premise: the e2e generator must refuse this step, got:\n{e2e}"
    );
    assert!(
        !snippet.contains("headings()[0]") && !snippet.contains("headings()?[0]"),
        "the snippet must not index a RustString leaf, got:\n{snippet}"
    );
    assert!(
        snippet.contains("result.metadata().headings()"),
        "the snippet must still show the bridged leaf itself, got:\n{snippet}"
    );
}

/// An `iterate` reads elements off its own leaf, so a bridged leaf has no shorter prefix that
/// still iterates — the operation goes, and the snippet falls back to the whole result.
#[test]
fn should_drop_an_iterate_over_a_json_bridged_leaf() {
    let fixture = fixture_iterating("metadata.headings", "section", &["text"]);
    let snippet = render_snippet(&fixture);

    assert!(
        !snippet.contains("for section in"),
        "the snippet must not iterate a RustString leaf, got:\n{snippet}"
    );
    assert!(
        snippet.contains("print(result)"),
        "dropping the only operation must fall back to showing the whole result, got:\n{snippet}"
    );
}

/// Negative control. `sections` is a genuine `RustVec`, so both generators must keep stepping into
/// it; a fix that refused every subscript would fail here.
#[test]
fn should_leave_a_countable_vec_leaf_indexable_in_both_generators() {
    let fixture = fixture_showing("metadata.sections[0].text");
    let e2e = render_e2e(&fixture);
    let snippet = render_snippet(&fixture);

    assert!(
        !e2e.contains(JSON_BRIDGE_SKIP),
        "premise: a countable RustVec leaf must not be refused, got:\n{e2e}"
    );
    assert!(
        snippet.contains("result.metadata().sections()[0].text()"),
        "the snippet must keep indexing a countable RustVec leaf, got:\n{snippet}"
    );
}

/// The invariant the preceding tests are instances of, stated once over a table: whenever the e2e
/// generator refuses a step for the JSON-bridge reason, the snippet generator must not spell that
/// step either — and whenever it does not refuse, the snippet must keep it.
#[test]
fn should_agree_with_the_e2e_generator_about_every_step_past_a_leaf() {
    let cases = [
        ("metadata.labels[\"theme\"]", "labels()"),
        ("metadata.headings[0].text", "headings()"),
        ("metadata.sections[0].text", "sections()"),
    ];
    for (path, accessor) in cases {
        let fixture = fixture_showing(path);
        let e2e = render_e2e(&fixture);
        let snippet = render_snippet(&fixture);
        let e2e_refuses = e2e.contains(JSON_BRIDGE_SKIP);
        let snippet_steps_past =
            snippet.contains(&format!("{accessor}[")) || snippet.contains(&format!("{accessor}?["));
        assert_eq!(
            e2e_refuses, !snippet_steps_past,
            "the two generators disagree about `{path}`: e2e refuses = {e2e_refuses}, \
             snippet steps past = {snippet_steps_past}\n--- e2e ---\n{e2e}\n--- snippet ---\n{snippet}"
        );
    }
}
