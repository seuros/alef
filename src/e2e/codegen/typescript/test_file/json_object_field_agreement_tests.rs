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
//! both callers share (see the `~keep` comment in `builders/mod.rs`), refusing (panicking
//! generation) rather than silently dropping an undeclared key. These tests pin both directions
//! at both call sites so the two renderers cannot drift apart again.

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
}

#[test]
fn an_undeclared_key_is_refused_identically_in_both_renderings() {
    // `SampleOptions` never declares `bogus` — a fixture typo, or a field the IR dropped.
    let fixture = fixture_with(serde_json::json!({"options": {"content": "hello", "bogus": "oops"}}));

    let snippet_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_snippet(&fixture)));
    assert!(
        snippet_result.is_err(),
        "snippet rendering must refuse an undeclared field instead of silently emitting it"
    );

    let e2e_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_e2e(&fixture)));
    assert!(
        e2e_result.is_err(),
        "e2e rendering must refuse the SAME undeclared field the snippet path refuses — a silent \
         drop here (while the snippet still refuses) is exactly the two-generators-disagree shape \
         #322 fixed"
    );
}

// SCRATCH probe (#337): does an undeclared key survive when it's nested one level deeper,
// inside an already-typed struct field reached via `node_value_expression` rather than through
// `ts_builder_expression_inner` directly? Not a permanent test.
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
fn scratch_probe_nested_undeclared_key() {
    let fixture = fixture_with(serde_json::json!({
        "options": {"content": "hello", "inner": {"known": "x", "bogus": "y"}}
    }));

    let snippet_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_snippet_nested(&fixture)));
    let e2e_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_e2e_nested(&fixture)));

    match (&snippet_result, &e2e_result) {
        (Ok(s), Ok(e)) => panic!("NEITHER panicked (bug reproduces): snippet=\n{s}\n\ne2e=\n{e}"),
        (Err(_), Err(_)) => panic!("BOTH panicked (already filtered, no gap)"),
        (Ok(_), Err(_)) => panic!("only snippet panicked"),
        (Err(_), Ok(_)) => panic!("only e2e panicked"),
    }
}
