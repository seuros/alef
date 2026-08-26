//! Regression coverage for a nested `Option<Vec<T>>` segment field reached through an
//! array-projected path (`entries[0].sections`) rather than as a top-level result field.
//!
//! Mirrors the Rust e2e generator's fix (`rust/test_file/test_function/optional_segment_len_tests.rs`):
//! `with_ir_fields` only ever proves a BARE field name optional ("sections"), by unanimity
//! across every declaration of that name in the crate — it has no path context. The
//! `render_kotlin_with_optionals` renderer keys its per-segment `?` safe-call check by the FULL
//! cumulative path walked so far ("entries[0].sections"), so a bare name never matches once the
//! path crosses more than one segment. `call_field_resolver::build_call_field_resolver` now
//! calls `with_anchored_optional_paths` with the fixture's own assertion field paths, the same
//! fix `presentation.rs` already applied for doc snippets.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_method::render_test_method;

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
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "get_report".into();
    e2e_config.call.result_var = "result".into();

    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture,
        "Facade",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        false,
        &ResolvedCrateConfig::default(),
        &sample_lib_type_defs(),
        &[],
        &sample_lib_functions(),
    )
    .expect("render_test_method succeeds");
    out
}

/// The confirmed defect, indexing site: `entries[0].sections[0].label` must safe-call (`?.`)
/// the `Option<List<Section>>` before indexing.
#[test]
fn nested_optional_segment_field_safe_calls_before_indexing() {
    let out = render(vec![make_assertion(
        "entries[0].sections[0].label",
        serde_json::Value::String("alpha".to_string()),
    )]);

    assert!(
        out.contains("sections()?.first()"),
        "must safe-call the Option<List<Section>> before indexing; got:\n{out}"
    );
    assert!(
        !out.contains("sections().first()"),
        "must not emit the un-guarded Option<List<T>> index; got:\n{out}"
    );
}

/// The confirmed defect, length site: `entries[0].sections.length` must safe-call before
/// `.size`.
#[test]
fn nested_optional_segment_field_safe_calls_before_size() {
    let out = render(vec![make_assertion(
        "entries[0].sections.length",
        serde_json::Value::Number(serde_json::Number::from(1u64)),
    )]);

    assert!(
        out.contains("sections()?.size"),
        "must safe-call the Option<List<Section>> before .size; got:\n{out}"
    );
    assert!(
        !out.contains("sections().size"),
        "must not emit .size directly against the nullable list; got:\n{out}"
    );
}

/// Negative control: `entries[0].tags` is a plain `Vec<String>` (not `Option<Vec<T>>`) on the
/// SAME owning type (`Entry`) as the optional `sections` field above. A fix that adds `?.`
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
        out.contains("tags().first()") && !out.contains("tags()?.first()"),
        "a non-optional Vec<T> field must index plainly, with no safe-call; got:\n{out}"
    );
    assert!(
        out.contains("tags().size") && !out.contains("tags()?.size"),
        "a non-optional Vec<T> field must read .size plainly, with no safe-call; got:\n{out}"
    );
}
