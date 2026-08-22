//! Regression coverage for the Kotlin e2e generator's collection-field classification.
//!
//! `render_assertion`'s `not_empty` arm on an OPTIONAL field only emits a real emptiness check
//! (`{field}?.isNotEmpty() == true`) when `field_is_collection` is true; otherwise it degrades to
//! a bare null-check (`{field} != null`), which passes for an empty-but-non-null collection —
//! silently missing the case the assertion exists to catch. `field_is_collection` comes from
//! `FieldResolver::is_array`/`is_collection_root`, both purely `fields_array`/`fields_optional`
//! config-derived. A collection field with NO per-element path declared anywhere in the fixture
//! suite (nothing ever indexes into it — e.g. a recursive `Option<List<DataNode>> Children`
//! field) has no config signal at all, so it fell through to the weaker null-check.
//!
//! `test_method.rs` now wires the same IR-derived collection classification the C# e2e generator
//! uses (`FieldResolver::ir_collection_fields` + `with_ir_collection_map`, anchored at the call's
//! declared Rust return type) so a field renders as a collection whenever the IR says so, config
//! or not. These tests drive the real entry point, `render_test_method`, with no
//! `fields_array`/`fields_optional` config at all — the classification must come from the IR
//! alone, mirroring `enum_field_classification_tests.rs` exactly.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_method::render_test_method;

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

fn fixture_calling(call: &str) -> Fixture {
    Fixture {
        id: "children_smoke".to_string(),
        description: "Children field smoke".to_string(),
        call: Some(call.to_string()),
        assertions: vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("children".to_string()),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

/// `field_is_optional` (kotlin/assertions.rs) is driven by `FieldResolver::is_optional`, which
/// in turn reads `e2e_config.effective_fields_optional` — a SEPARATE config surface from the IR
/// collection classification under test. Declaring it here isolates the one variable this suite
/// actually exercises: optionality is a config given, collection-ness is the IR-derived fact
/// being tested.
fn e2e_config_for(call: &str) -> E2eConfig {
    let call_config = CallConfig {
        function: call.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config.fields_optional.insert("children".to_string());
    e2e_config
}

fn render(fixture: &Fixture, e2e_config: &E2eConfig, type_defs: &[TypeDef], functions: &[FunctionDef]) -> String {
    let mut out = String::new();
    render_test_method(
        &mut out,
        fixture,
        "Facade",
        "",
        "",
        &[],
        None,
        false,
        e2e_config,
        &std::collections::HashMap::new(),
        false,
        &ResolvedCrateConfig::default(),
        type_defs,
        &[],
        functions,
    )
    .expect("render_test_method succeeds");
    out
}

/// Regression: `not_empty` on an undeclared `Option<List<DataNode>> Children` field must render
/// `?.isNotEmpty() == true`, never the bare `!= null` check that also passes for an
/// empty-but-non-null collection.
#[test]
fn not_empty_on_an_undeclared_optional_collection_field_is_classified_via_the_ir() {
    let (type_defs, functions) = table_ir();
    let e2e_config = e2e_config_for("process");
    let fixture = fixture_calling("process");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("isNotEmpty() == true"),
        "an undeclared optional collection field must render a real emptiness check, got:\n{out}"
    );
    assert!(
        !out.contains("children() != null"),
        "must not degrade to a bare null-check that also passes for an empty collection, got:\n{out}"
    );
}

/// A plain optional `String` field with the same name on an unrelated type must not be
/// misclassified as a collection — the IR classification is anchored per-call, not matched on
/// the leaf name alone. A `String?` field keeps the bare null-check: `.isNotEmpty()` on a
/// `String?` receiver would need a different (non-collection) safe-call shape.
#[test]
fn a_same_named_optional_string_field_on_an_unrelated_type_is_not_misclassified_as_a_collection() {
    let (type_defs, functions) = table_ir();
    let e2e_config = e2e_config_for("other");
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &functions);
    assert!(
        out.contains("children() != null"),
        "a plain optional string field's not_empty must keep the bare null-check, got:\n{out}"
    );
    assert!(
        !out.contains("isNotEmpty() == true"),
        "a plain optional string field must not take the collection branch, got:\n{out}"
    );
}
