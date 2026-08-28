//! Snippet vs e2e agreement on a `json_object` argument's declared fields (#322).
//!
//! `render_snippet_body` (docs snippets, `snippet.rs`) and `render_test_case` (the generated
//! vitest suite, `test_case.rs`) both build their object literal through the same
//! `ts_builder_expression` / `build_args_and_setup` machinery, but historically checked it under
//! different TypeScript strictness: the snippet path binds the literal to a typed `const`
//! (`typed_binding.jinja`), which `tsc` DOES excess-property-check, while the e2e path only ever
//! `as`-casts the same literal, which `tsc` does NOT excess-property-check. A fixture with a key
//! the bound type doesn't declare therefore compiled clean as a generated test and failed
//! (TS2353) only in the published snippet for the identical input.
//!
//! The fix filters the object literal's keys against the type's declared fields at the one site
//! both callers share (see the `~keep` comment in `builders/mod.rs`), refusing rather than
//! silently dropping an undeclared key. The refusal is recorded on `fixture_refusal`'s ledger
//! and becomes the backend's own `Err` at `E2eCodegen::generate_gated`; it deliberately does not
//! unwind, so these tests read the ledger rather than catching a panic. These tests pin both
//! directions at both call sites so the two renderers cannot drift apart again.

use super::snippet::{SnippetContext, render_snippet_body};
use super::test_case::render_test_case;
use super::tests::{make_field, make_type};
use crate::core::ir::TypeRef;
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::Fixture;

fn options_arg() -> ArgMapping {
    ArgMapping {
        name: "options".into(),
        field: "input.options".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("SampleOptions".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn e2e_config() -> E2eConfig {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process_thing".into();
    e2e_config.call.module = "@example/library".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.args = vec![options_arg()];
    e2e_config
}

/// `SampleOptions` declares exactly one field, `content` — anything else in a fixture's
/// `options` object is undeclared.
fn option_type_defs() -> Vec<crate::core::ir::TypeDef> {
    vec![make_type("SampleOptions", vec![make_field("content", TypeRef::String)])]
}

fn fixture_with(input: serde_json::Value) -> Fixture {
    Fixture {
        id: "process_thing".into(),
        description: "Process a thing".into(),
        input,
        ..Fixture::default()
    }
}

fn render_snippet(fixture: &Fixture) -> String {
    let config = crate::core::config::ResolvedCrateConfig::default();
    render_snippet_body(SnippetContext {
        lang: "node",
        fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config(),
        type_defs: &option_type_defs(),
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    })
}

fn render_e2e(fixture: &Fixture) -> String {
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs = option_type_defs();
    let mut out = String::new();
    let mut referenced_enums = std::collections::BTreeSet::new();
    render_test_case(
        &mut out,
        fixture,
        None,
        None,
        &e2e_config(),
        "node",
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        &[],
        "",
        &config,
        &mut referenced_enums,
        &[],
    );
    out
}

#[test]
fn a_declared_key_survives_identically_in_both_renderings() {
    let fixture = fixture_with(serde_json::json!({"options": {"content": "hello"}}));

    let snippet = render_snippet(&fixture);
    assert!(
        snippet.contains(r#"content: "hello""#),
        "snippet must build the declared field:\n{snippet}"
    );

    let e2e = render_e2e(&fixture);
    assert!(
        e2e.contains(r#"content: "hello""#),
        "e2e test must build the declared field:\n{e2e}"
    );
    // ~keep Proves the guard is not refusing everything: a check that fired on correct input
    // would make every assertion above pass for the wrong reason.
    assert_eq!(
        crate::e2e::codegen::fixture_refusal::take().len(),
        0,
        "a fully declared fixture must record no refusal"
    );
}

/// Drain the refusal ledger and return the composed diagnostic, or `None` when nothing was
/// refused. Both renderers below attribute their own refusals before returning, so this reads
/// whatever the render just recorded.
fn refusal() -> Option<String> {
    crate::e2e::codegen::fixture_refusal::take_error("node").map(|error| format!("{error:#}"))
}

#[test]
fn an_undeclared_key_is_refused_identically_in_both_renderings() {
    // `SampleOptions` never declares `bogus` — a fixture typo, or a field the IR dropped.
    let fixture = fixture_with(serde_json::json!({"options": {"content": "hello", "bogus": "oops"}}));

    render_snippet(&fixture);
    let snippet_refusal =
        refusal().expect("snippet rendering must refuse an undeclared field instead of silently emitting it");
    assert!(snippet_refusal.contains("bogus"), "got: {snippet_refusal}");

    render_e2e(&fixture);
    let e2e_refusal = refusal().expect(
        "e2e rendering must refuse the SAME undeclared field the snippet path refuses — a silent \
         drop here (while the snippet still refuses) is exactly the two-generators-disagree shape \
         #322 fixed",
    );
    assert!(e2e_refusal.contains("bogus"), "got: {e2e_refusal}");
}

/// The refusal both renderers raise must name the fixture, the language and the `options_type`
/// lever -- the message's whole job. Naming only the type and the key sent an operator to "fix
/// the fixture or the Rust struct" in an incident where both were already correct. ~keep
#[test]
fn the_refusal_names_the_call_the_language_and_the_options_type_lever() {
    let fixture = fixture_with(serde_json::json!({"options": {"content": "hello", "bogus": "oops"}}));

    render_e2e(&fixture);
    let message = refusal().expect("an undeclared key must be refused");

    assert!(message.contains("fixture `process_thing`"), "got: {message}");
    assert!(message.contains("language `node`"), "got: {message}");
    assert!(
        message.contains("options_type"),
        "the message must point at the per-language `options_type` override as a lever: {message}"
    );
    assert!(
        message.contains("`[e2e.call.overrides.node]`"),
        "the message must name the override table to edit: {message}"
    );
}

/// `SampleOptions.inner` is a SEPARATE struct type (`InnerOptions`, one declared field:
/// `known`) reached one level deeper than the top-level `options` argument. Object literals at
/// this depth are built by `node_value_expression`, not `ts_builder_expression_inner` — a
/// distinct function from the one the tests above exercise. Both must apply the same
/// undeclared-key guard (see `refuse_undeclared_json_keys` in `builders/mod.rs`), or the same
/// snippet-vs-e2e asymmetry these top-level tests pin can still occur one level deeper.
fn option_type_defs_with_nested() -> Vec<crate::core::ir::TypeDef> {
    vec![
        make_type(
            "SampleOptions",
            vec![
                make_field("content", TypeRef::String),
                make_field("inner", TypeRef::Named("InnerOptions".into())),
            ],
        ),
        make_type("InnerOptions", vec![make_field("known", TypeRef::String)]),
    ]
}

fn render_snippet_nested(fixture: &Fixture) -> String {
    let config = crate::core::config::ResolvedCrateConfig::default();
    render_snippet_body(SnippetContext {
        lang: "node",
        fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config(),
        type_defs: &option_type_defs_with_nested(),
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    })
}

fn render_e2e_nested(fixture: &Fixture) -> String {
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs = option_type_defs_with_nested();
    let mut out = String::new();
    let mut referenced_enums = std::collections::BTreeSet::new();
    render_test_case(
        &mut out,
        fixture,
        None,
        None,
        &e2e_config(),
        "node",
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        &[],
        "",
        &config,
        &mut referenced_enums,
        &[],
    );
    out
}

#[test]
fn a_nested_declared_key_survives_identically_in_both_renderings() {
    let fixture = fixture_with(serde_json::json!({"options": {"content": "hello", "inner": {"known": "x"}}}));

    let snippet = render_snippet_nested(&fixture);
    assert!(
        snippet.contains(r#"known: "x""#),
        "snippet must build the declared nested field:\n{snippet}"
    );

    let e2e = render_e2e_nested(&fixture);
    assert!(
        e2e.contains(r#"known: "x""#),
        "e2e test must build the declared nested field:\n{e2e}"
    );
}

#[test]
fn a_nested_undeclared_key_is_refused_identically_in_both_renderings() {
    // `InnerOptions` (the type of `SampleOptions.inner`) never declares `bogus`. This key is
    // nested one level deeper than the top-level `options` argument, so it reaches
    // `node_value_expression`'s object-literal builder rather than
    // `ts_builder_expression_inner`'s.
    let fixture =
        fixture_with(serde_json::json!({"options": {"content": "hello", "inner": {"known": "x", "bogus": "y"}}}));

    render_snippet_nested(&fixture);
    let snippet_refusal =
        refusal().expect("snippet rendering must refuse an undeclared nested field instead of silently emitting it");
    assert!(
        snippet_refusal.contains("reached through field `inner`"),
        "a nested refusal must say where it sat: {snippet_refusal}"
    );

    render_e2e_nested(&fixture);
    let e2e_refusal = refusal().expect(
        "e2e rendering must refuse the SAME undeclared nested field the snippet path refuses — a \
         silent drop here (while the snippet still refuses) reproduces the two-generators-disagree \
         shape one level deeper than the top-level fix covers",
    );
    assert!(e2e_refusal.contains("bogus"), "got: {e2e_refusal}");
}
