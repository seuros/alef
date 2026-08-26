//! Regression coverage for a nested `Option<Vec<T>>` segment field reached through an
//! array-projected path (`entries[0].sections`) rather than as a top-level result field.
//!
//! Mirrors the Rust e2e generator's fix (`rust/test_file/test_function/optional_segment_len_tests.rs`):
//! `with_ir_fields` only ever proves a BARE field name optional ("sections"), by unanimity
//! across every declaration of that name in the crate — it has no path context. The
//! `render_typescript_with_optionals` renderer keys its per-segment `?.` check by the FULL
//! cumulative path walked so far ("entries[0].sections"), so a bare name never matches once the
//! path crosses more than one segment. `test_case.rs::render_test_case` now calls
//! `with_anchored_optional_paths` with the fixture's own assertion field paths, the same fix
//! `presentation.rs` already applied for doc snippets.

use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

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
        rust_path: "sample_lib::get_report".to_string(),
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
    e2e_config.call.module = "sample-lib".into();
    e2e_config.call.result_var = "result".into();

    let config = crate::core::config::ResolvedCrateConfig::default();
    let mut out = String::new();
    let mut referenced_enums = std::collections::BTreeSet::new();
    super::test_case::render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        "node",
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &sample_lib_type_defs(),
        &[],
        &sample_lib_functions(),
        "",
        &config,
        &mut referenced_enums,
        &[],
    );
    out
}

/// The confirmed defect, indexing site: `entries[0].sections[0].label` must optional-chain
/// (`?.`) the `Option<Vec<Section>>` before indexing.
#[test]
fn nested_optional_segment_field_optional_chains_before_indexing() {
    let out = render(vec![make_assertion(
        "entries[0].sections[0].label",
        serde_json::Value::String("alpha".to_string()),
    )]);

    assert!(
        out.contains("sections?.[0]"),
        "must optional-chain the Option<Vec<Section>> before indexing; got:\n{out}"
    );
    assert!(
        !out.contains("sections[0]"),
        "must not emit the un-guarded Option<Vec<T>> index; got:\n{out}"
    );
}

/// The confirmed defect, length site: `entries[0].sections.length` must optional-chain before
/// `.length`.
#[test]
fn nested_optional_segment_field_optional_chains_before_length() {
    let out = render(vec![make_assertion(
        "entries[0].sections.length",
        serde_json::Value::Number(serde_json::Number::from(1u64)),
    )]);

    assert!(
        out.contains("sections?.length"),
        "must optional-chain the Option<Vec<Section>> before .length; got:\n{out}"
    );
    assert!(
        !out.contains("sections.length"),
        "must not emit .length directly against the possibly-undefined array; got:\n{out}"
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
        out.contains("tags[0]") && !out.contains("tags?.[0]"),
        "a non-optional Vec<T> field must index plainly, with no optional chaining; got:\n{out}"
    );
    assert!(
        out.contains("tags.length") && !out.contains("tags?.length"),
        "a non-optional Vec<T> field must read .length plainly, with no optional chaining; got:\n{out}"
    );
}
