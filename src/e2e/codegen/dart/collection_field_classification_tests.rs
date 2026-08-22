//! Regression coverage for the Dart e2e generator's collection-field classification.
//!
//! `render_assertion_dart`'s `is_empty`/`not_empty` arms decide whether a field has an
//! `.isEmpty` getter (a real collection check) purely from `FieldResolver::is_array` — config-
//! derived from `fields_array` alone. A collection field with NO per-element path declared
//! anywhere in the fixture suite (nothing ever indexes into it — e.g. a recursive
//! `Option<Vec<DataNode>> Children`) has no config signal at all, so it fell through to the
//! `.toString()`-based non-collection branch: `expect((result.children?.toString() ?? ''),
//! isEmpty)` compares against the Dart `List.toString()` representation (`'[]'` for an empty
//! list — never actually empty as a string), so the assertion could never pass.
//!
//! `test_case.rs` now wires the same IR-derived collection classification csharp/kotlin already
//! use (`FieldResolver::ir_collection_fields` + `with_ir_collection_map`, anchored at the call's
//! declared Rust return type) so a field renders as a collection whenever the IR says so, config
//! or not. These tests drive the real entry point, `render_test_case`, with no
//! `fields_array`/`fields_optional` config at all — the classification must come from the IR
//! alone, mirroring `csharp/collection_field_classification_tests.rs` and
//! `kotlin/collection_field_classification_tests.rs`. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::DartFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_case::{DartTestCaseContext, render_test_case};

fn children_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "children".to_string(),
        ty: TypeRef::Optional(Box::new(ty)),
        ..FieldDef::default()
    }
}

fn table_ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![children_field(TypeRef::Vec(Box::new(TypeRef::Named(
                "DataNode".to_string(),
            ))))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OtherResult".to_string(),
            fields: vec![children_field(TypeRef::String)],
            ..TypeDef::default()
        },
    ];
    let functions = vec![
        FunctionDef {
            name: "process".to_string(),
            return_type: TypeRef::Named("ProcessResult".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "other".to_string(),
            return_type: TypeRef::Named("OtherResult".to_string()),
            ..FunctionDef::default()
        },
    ];
    (type_defs, functions)
}

fn fixture_calling_with_assertion(call: &str, assertion_type: &str) -> Fixture {
    Fixture {
        id: "children_smoke".to_string(),
        description: "Children field smoke".to_string(),
        call: Some(call.to_string()),
        assertions: vec![Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some("children".to_string()),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn e2e_config_for(call: &str) -> E2eConfig {
    let call_config = CallConfig {
        function: call.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config
}

fn render(fixture: &Fixture, e2e_config: &E2eConfig, type_defs: &[TypeDef], functions: &[FunctionDef]) -> String {
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let dart_first_class_map = DartFirstClassMap::default();
    let mut out = String::new();
    render_test_case(
        &mut out,
        fixture,
        DartTestCaseContext {
            e2e_config,
            lang: "dart",
            bridge_class: "Sample",
            dart_first_class_map: &dart_first_class_map,
            adapters: &[],
            config: &config,
            type_defs,
            enums: &[],
            functions,
            errors: &[],
            native_typed_dtos: true,
            is_snippet: false,
        },
    );
    out
}

/// Regression: `not_empty` on an undeclared `Option<Vec<DataNode>> Children` field must render
/// `expect(..., isNotEmpty)` directly on the collection, never the `.toString()`-based check
/// that can never pass on a non-string collection.
#[test]
fn not_empty_on_an_undeclared_optional_collection_field_is_classified_via_the_ir() {
    let (type_defs, functions) = table_ir();
    let e2e_config = e2e_config_for("process");
    let fixture = fixture_calling_with_assertion("process", "not_empty");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("children, isNotEmpty)"),
        "an undeclared optional collection field must render a real isNotEmpty check on the \
         collection itself, got:\n{out}"
    );
    assert!(
        !out.contains("toString(), isNotEmpty)"),
        "must not degrade to the toString()-based check, which compares against Dart's \
         `List.toString()` representation and can never pass, got:\n{out}"
    );
}

/// Regression: `is_empty` on the same field must render `expect(..., anyOf(isNull, isEmpty))`
/// directly on the collection, never the `.toString()`-based check (`'[]'` is a 2-character
/// non-empty string, so that check can never pass either).
#[test]
fn is_empty_on_an_undeclared_optional_collection_field_is_classified_via_the_ir() {
    let (type_defs, functions) = table_ir();
    let e2e_config = e2e_config_for("process");
    let fixture = fixture_calling_with_assertion("process", "is_empty");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("children, anyOf(isNull, isEmpty))"),
        "an undeclared optional collection field must render a real anyOf(isNull, isEmpty) \
         check on the collection itself, got:\n{out}"
    );
    assert!(
        !out.contains("toString() ?? ''), isEmpty)"),
        "must not degrade to the toString()-based check, which sees '[]' for an empty list and \
         can never pass, got:\n{out}"
    );
}

/// A plain optional `String` field with the same name on an unrelated type must not be
/// misclassified as a collection — the IR classification is anchored per-call, not matched on
/// the leaf name alone.
#[test]
fn a_same_named_optional_string_field_on_an_unrelated_type_is_not_misclassified_as_a_collection() {
    let (type_defs, functions) = table_ir();
    let e2e_config = e2e_config_for("other");
    let fixture = fixture_calling_with_assertion("other", "is_empty");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("toString() ?? ''), isEmpty)"),
        "a plain optional string field's is_empty must keep the toString()-based check, got:\n{out}"
    );
    assert!(
        !out.contains("children, anyOf(isNull, isEmpty))"),
        "a plain optional string field must not take the collection branch, got:\n{out}"
    );
}
