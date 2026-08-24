//! A Java enhanced-for binding may not be named after the collection it iterates.
//!
//! `for (var result : result.results())` is a redeclaration, not a shadow: the loop variable is
//! declared in the same method scope the result local already occupies, so `javac` rejects it with
//! "variable result is already defined". See `codegen::loop_binding` for where the name is decided;
//! this file checks the Java snippet path routes through it. ~keep

use super::snippet::render_snippet_body;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureDocs, FixtureDocsOperation, FixtureDocsPresentation, SideEffectClass};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn listing_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "SampleListing".to_string(),
            fields: vec![field(
                "results",
                TypeRef::Vec(Box::new(TypeRef::Named("SampleEntry".to_string()))),
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "SampleEntry".to_string(),
            fields: vec![field("text", TypeRef::String)],
            ..TypeDef::default()
        },
    ]
}

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
        }),
        ..Fixture::default()
    }
}

fn snippet_for(item: &str) -> String {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "list_entries".into();
    e2e_config.call.result_var = "result".into();
    let fixture = listing_fixture(item);
    render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &listing_types())
}

fn loop_line(body: &str) -> String {
    body.lines()
        .find(|line| line.trim_start().starts_with("for ("))
        .unwrap_or_else(|| panic!("snippet must iterate:\n{body}"))
        .trim()
        .to_string()
}

#[test]
fn a_loop_binding_never_shadows_the_collection_it_iterates() {
    let body = snippet_for("result");
    assert_eq!(
        loop_line(&body),
        "for (var item : result.results()) {",
        "the loop binding must not reuse the result local it iterates:\n{body}"
    );
    assert!(
        body.contains("System.out.println(item.text());"),
        "the per-item field access must follow the renamed binding:\n{body}"
    );
}

/// Negative control: a binding that collides with nothing keeps the name the fixture authored.
#[test]
fn a_loop_binding_that_collides_with_nothing_keeps_its_authored_name() {
    let body = snippet_for("entry");
    assert_eq!(
        loop_line(&body),
        "for (var entry : result.results()) {",
        "an authored binding that shadows nothing must survive:\n{body}"
    );
    assert!(
        body.contains("System.out.println(entry.text());"),
        "the per-item field access must follow the authored binding:\n{body}"
    );
}
