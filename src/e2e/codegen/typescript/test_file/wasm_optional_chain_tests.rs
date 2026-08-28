//! `node` and `wasm` are the same TypeScript surface, so they must spell an optional field
//! access the same way.
//!
//! Both targets compile the snippet they emit with the same `strict` TypeScript, so a
//! divergence here is not a style difference: an unguarded member access on a possibly-absent
//! value is `TS18048`, and the snippet validator fails on it. This file is the agreement
//! check, kept out of `snippet.rs` and `tests.rs` (both remediation targets) because it owns a
//! single question with its own fixture. ~keep

use super::snippet::{SnippetContext, render_snippet_body};
use super::tests::{make_field, make_type};
use crate::core::ir::{FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureDocs};

/// A result type with two struct fields of the same type -- one genuinely `Option<T>`, one
/// required -- whose own field is a plain `Vec<String>`. So the only thing that can make an
/// accessor on the leaf need guarding is the link above it, and the required twin is the
/// control that an unconditional guard would break.
fn analysis_types() -> Vec<TypeDef> {
    let mut document = make_field(
        "document",
        TypeRef::Optional(Box::new(TypeRef::Named("Section".into()))),
    );
    document.optional = true;
    let summary = make_field("summary", TypeRef::Named("Section".into()));
    let mut report = make_type("Report", vec![document, summary]);
    // `has_default` is what widens every field of a node/NAPI binding to optional. Left off so
    // the agreement this file checks is about the field's own declared `Option<T>`, not about
    // the NAPI widening rule (which applies to node and not to wasm, by design). ~keep
    report.has_default = false;
    let mut section = make_type(
        "Section",
        vec![make_field("nodes", TypeRef::Vec(Box::new(TypeRef::String)))],
    );
    section.has_default = false;
    vec![report, section]
}

fn analysis_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "analyze".into(),
        rust_path: "sample::analyze".into(),
        return_type: TypeRef::Named("Report".into()),
        ..FunctionDef::default()
    }]
}

fn shows_fixture(path: &str) -> Fixture {
    Fixture {
        id: "analyze_document".into(),
        description: "Analyze a document".into(),
        input: serde_json::Value::Null,
        docs: Some(FixtureDocs {
            topic: "guides".into(),
            stem: None,
            paths: Default::default(),
            title: None,
            description: None,
            input: None,
            shows: vec![path.to_string()],
            error: None,
            presentation: None,
            client: None,
            side_effects: crate::e2e::fixture::SideEffectClass::Safe,
            coverage_exceptions: Default::default(),
            sample_url_vars: Default::default(),
            body_file: None,
        }),
        ..Fixture::default()
    }
}

fn snippet_for(lang: &str, path: &str) -> String {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "analyze".into();
    e2e_config.call.module = "@example/library".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.r#async = true;
    let fixture = shows_fixture(path);
    let config = crate::core::config::ResolvedCrateConfig::default();
    render_snippet_body(SnippetContext {
        lang,
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config,
        type_defs: &analysis_types(),
        enums: &[],
        functions: &analysis_functions(),
        wasm_type_prefix: "",
        config: &config,
    })
}

/// The line the snippet prints the shown path on, which is the whole observable difference.
fn shown_expression(body: &str) -> String {
    body.lines()
        .find(|line| line.trim_start().starts_with("console.log(result."))
        .unwrap_or_else(|| panic!("snippet must show the result field:\n{body}"))
        .trim()
        .to_string()
}

#[test]
fn node_and_wasm_guard_an_optional_field_the_same_way() {
    let node = shown_expression(&snippet_for("node", "document.nodes"));
    let wasm = shown_expression(&snippet_for("wasm", "document.nodes"));

    assert_eq!(
        node, wasm,
        "node and wasm are one TypeScript surface and must emit one accessor"
    );
    assert_eq!(
        wasm, "console.log(result.document?.nodes);",
        "an access through an `Option<T>` field must be optional-chained, or the emitted \
         snippet is a TS18048 under the strict TypeScript the validator compiles with"
    );
}

/// Negative control: `?.` on a field that is not optional is its own (milder) defect, and an
/// "always chain" fix would pass the test above while failing this one. `summary` is the same
/// `Section` type reached through a required link instead of an optional one.
#[test]
fn neither_target_guards_a_field_that_is_not_optional() {
    let node = shown_expression(&snippet_for("node", "summary.nodes"));
    let wasm = shown_expression(&snippet_for("wasm", "summary.nodes"));

    assert_eq!(node, wasm, "node and wasm must agree on the unguarded case too");
    assert_eq!(
        wasm, "console.log(result.summary.nodes);",
        "a chain with no optional link must not gain a guard"
    );
}
