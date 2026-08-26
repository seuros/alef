//! Regression coverage for a nested `Option<Vec<T>>` segment field reached through an
//! array-projected path (`Entries[0].Sections`) rather than as a top-level result field.
//!
//! Mirrors the Rust e2e generator's `rust/test_file/test_function/optional_segment_len_tests.rs`
//! fix: `with_ir_fields` only ever proves a BARE field name optional ("sections"), by unanimity
//! across every declaration of that name in the crate — it has no path context. The C#
//! `render_csharp_with_optionals` renderer keys its per-segment null-forgiving (`!`) check by the
//! FULL cumulative path walked so far ("entries[0].sections"), so a bare name never matches once
//! the path crosses more than one segment. `call_field_resolver::build_call_field_resolver` now
//! calls `with_anchored_optional_paths` with the fixture's own assertion field paths, the same
//! fix `presentation.rs` already applied for doc snippets.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

fn sample_lib_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "SampleResult".to_string(),
            fields: vec![FieldDef {
                name: "entries".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Entry".to_string(),
            fields: vec![
                FieldDef {
                    name: "sections".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Section".to_string()))),
                    optional: true,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "tags".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    optional: false,
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Section".to_string(),
            fields: vec![FieldDef {
                name: "label".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn sample_lib_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "get_report".to_string(),
        return_type: TypeRef::Named("SampleResult".to_string()),
        ..FunctionDef::default()
    }]
}

fn make_assertion(field: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    }
}

fn render(assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "get_report_sections".into(),
        description: "sample_lib report with optional nested sections".into(),
        assertions,
        ..Fixture::default()
    };
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "get_report".into();
    e2e_config.call.result_var = "result".into();

    let placeholder_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample_lib".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        &fixture,
        "SampleResultClient",
        "GetReport",
        "SampleLibException",
        "result",
        &[],
        &placeholder_resolver,
        false,
        false,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        &sample_lib_type_defs(),
        &[],
        &sample_lib_functions(),
        &[],
    );
    out
}

/// The confirmed defect, indexing site: `Entries[0].Sections[0].Label` must null-forgive
/// (`!`) the `Option<Vec<Section>>` before indexing.
#[test]
fn nested_optional_segment_field_null_forgives_before_indexing() {
    let out = render(vec![make_assertion(
        "entries[0].sections[0].label",
        serde_json::Value::String("alpha".to_string()),
    )]);

    assert!(
        out.contains("Sections![0]"),
        "must null-forgive the Option<Vec<Section>> before indexing; got:\n{out}"
    );
    assert!(
        !out.contains("Sections[0]"),
        "must not emit the un-null-forgiven Option<Vec<T>> index; got:\n{out}"
    );
}

/// The confirmed defect, length site: `Entries[0].Sections.Count` must null-forgive the
/// `Option<Vec<Section>>` before `.Count`.
#[test]
fn nested_optional_segment_field_null_forgives_before_count() {
    let out = render(vec![make_assertion(
        "entries[0].sections.length",
        serde_json::Value::Number(serde_json::Number::from(1u64)),
    )]);

    assert!(
        out.contains("Sections!.Count"),
        "must null-forgive the Option<Vec<Section>> before .Count; got:\n{out}"
    );
    assert!(
        !out.contains("Sections.Count"),
        "must not emit .Count directly against the nullable collection; got:\n{out}"
    );
}

/// Negative control: `Entries[0].Tags` is a plain `Vec<String>` (not `Option<Vec<T>>`) on the
/// SAME owning type (`Entry`) as the optional `sections` field above. A fix that adds `!`
/// unconditionally — rather than reading `FieldDef::optional` per field — is a different bug.
#[test]
fn non_optional_sibling_segment_field_stays_plain() {
    let out = render(vec![
        make_assertion("entries[0].tags[0]", serde_json::Value::String("beta".to_string())),
        make_assertion(
            "entries[0].tags.length",
            serde_json::Value::Number(serde_json::Number::from(1u64)),
        ),
    ]);

    assert!(
        out.contains("Tags[0]") && !out.contains("Tags![0]"),
        "a non-optional Vec<T> field must index plainly, with no null-forgiving operator; got:\n{out}"
    );
    assert!(
        out.contains("Tags.Count") && !out.contains("Tags!.Count"),
        "a non-optional Vec<T> field must read .Count plainly, with no null-forgiving operator; got:\n{out}"
    );
}
