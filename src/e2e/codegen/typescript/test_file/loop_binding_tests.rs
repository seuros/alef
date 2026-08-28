//! A TypeScript loop binding may not be named after the collection it iterates.
//!
//! `for (const result of result.results ?? [])` reads as a shadow but is not one: the head's
//! expression is evaluated inside the loop's own declarative environment, where `result` is
//! already bound and still in its temporal dead zone, so `tsc` rejects it with `TS2448`/`TS7022`
//! before the snippet validator ever runs it. See `codegen::loop_binding` for where the name is
//! decided; this file checks the node/wasm snippet path routes through it. ~keep

use super::snippet::{SnippetContext, render_snippet_body};
use super::tests::{make_field, make_type};
use crate::core::ir::{FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureDocs, FixtureDocsOperation, FixtureDocsPresentation, SideEffectClass};

fn listing_types() -> Vec<TypeDef> {
    let mut entry = make_type("SampleEntry", vec![make_field("text", TypeRef::String)]);
    entry.has_default = false;
    let mut listing = make_type(
        "SampleListing",
        vec![make_field(
            "results",
            TypeRef::Vec(Box::new(TypeRef::Named("SampleEntry".into()))),
        )],
    );
    listing.has_default = false;
    vec![listing, entry]
}

fn listing_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "list_entries".into(),
        rust_path: "sample::list_entries".into(),
        return_type: TypeRef::Named("SampleListing".into()),
        ..FunctionDef::default()
    }]
}

/// The authored shape that shipped: the loop binding carries the same name as the call's result.
fn listing_fixture(item: &str) -> Fixture {
    Fixture {
        id: "list_entries".into(),
        description: "List entries".into(),
        input: serde_json::Value::Null,
        docs: Some(FixtureDocs {
            topic: "guides".into(),
            stem: None,
            paths: Default::default(),
            title: None,
            description: None,
            input: None,
            shows: Vec::new(),
            error: None,
            presentation: Some(FixtureDocsPresentation {
                call: None,
                input: None,
                args: None,
                files: Vec::new(),
                operations: vec![FixtureDocsOperation::Iterate {
                    path: "results".into(),
                    item: item.into(),
                    fields: vec!["text".into()],
                    display: false,
                    optional: false,
                }],
            }),
            client: None,
            side_effects: SideEffectClass::Safe,
            coverage_exceptions: Default::default(),
            sample_url_vars: Default::default(),
        }),
        ..Fixture::default()
    }
}

fn snippet_for(lang: &str, item: &str) -> String {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "list_entries".into();
    e2e_config.call.module = "@example/library".into();
    e2e_config.call.result_var = "result".into();
    let fixture = listing_fixture(item);
    let config = crate::core::config::ResolvedCrateConfig::default();
    render_snippet_body(SnippetContext {
        lang,
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config,
        type_defs: &listing_types(),
        enums: &[],
        functions: &listing_functions(),
        wasm_type_prefix: "",
        config: &config,
    })
}

fn loop_line(body: &str) -> String {
    body.lines()
        .find(|line| line.trim_start().starts_with("for ("))
        .unwrap_or_else(|| panic!("snippet must iterate:\n{body}"))
        .trim()
        .to_string()
}

/// The loop binding must differ from the result variable its iterated expression reads.
#[test]
fn a_loop_binding_never_shadows_the_collection_it_iterates() {
    for lang in ["node", "wasm"] {
        let body = snippet_for(lang, "result");
        assert_eq!(
            loop_line(&body),
            "for (const item of result.results) {",
            "the {lang} loop binding must not reuse the result variable it iterates:\n{body}"
        );
        assert!(
            body.contains("console.log(item.text);"),
            "the per-item field access must follow the renamed binding:\n{body}"
        );
    }
}

/// Negative control: a binding that collides with nothing keeps the name the fixture authored, so
/// this is a collision fix rather than a blanket rename.
#[test]
fn a_loop_binding_that_collides_with_nothing_keeps_its_authored_name() {
    for lang in ["node", "wasm"] {
        let body = snippet_for(lang, "entry");
        assert_eq!(
            loop_line(&body),
            "for (const entry of result.results) {",
            "an authored binding that shadows nothing must survive:\n{body}"
        );
        assert!(
            body.contains("console.log(entry.text);"),
            "the per-item field access must follow the authored binding:\n{body}"
        );
    }
}
