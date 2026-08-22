//! Regression coverage for the Rust e2e generator's collection-field classification.
//!
//! Split into its own file rather than added to `rust/assertions.rs`: that file is already over
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test coverage goes
//! into a fresh module instead of growing it. ~keep
//!
//! `render_assertion`'s `contains`/`contains_all`/`not_contains` arms pick `containment_predicate`'s
//! collection arm (an `.iter().any(...)` scan) only when `field_is_collection` is true;
//! otherwise they emit `{field_access}.contains({expected})` directly, which requires `expected`
//! (a `&str`) to be the collection's OWN element type — for a `Vec<DataNode>` field this is a
//! Rust compile error (`expected DataNode, found &str`), not a runtime failure. `field_is_collection`
//! reads `FieldResolver::is_array`/`is_collection_root`, both purely `fields_array` config-
//! derived. A collection field with NO per-element path declared anywhere in the fixture suite
//! (nothing ever indexes into it — e.g. a recursive `Vec<DataNode> children` field) has no
//! config signal at all, so it fell through to the scalar `.contains(&str)` shape.
//!
//! `test_file/test_function.rs` now wires the same IR-derived collection classification the
//! C#/Kotlin/Swift e2e generators use (`FieldResolver::ir_collection_fields` +
//! `with_ir_collection_map`, anchored at the call's declared Rust return type) so a field
//! renders as a collection whenever the IR says so, config or not. These tests drive
//! `render_assertion` directly with an IR-anchored resolver and no `fields_array` config at
//! all — the classification must come from the IR alone, mirroring
//! `assertion_containment_tests.rs`'s direct-call style.

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn children_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "children".to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn table_ir() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "ProcessResult".to_string(),
        fields: vec![children_field(TypeRef::Vec(Box::new(TypeRef::Named(
            "DataNode".to_string(),
        ))))],
        ..TypeDef::default()
    }]
}

fn ir_anchored_resolver(type_defs: &[TypeDef], root_type: &str) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_collection_map(
        FieldResolver::ir_collection_fields(type_defs),
        Some(root_type.to_string()),
    )
}

fn render_contains(field_resolver: &FieldResolver, field: &str, expected: &str) -> String {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!(expected)),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        "sample",
        false,
        &[],
        field_resolver,
        false,
        false,
        false,
        false,
        false,
        None,
    );
    out
}

/// Regression: `contains` on an undeclared `Vec<DataNode> children` field must render the
/// per-element scan (compiles against any element type), never the scalar `.contains(&str)`
/// shape that requires the collection's own element type and would not compile against
/// `Vec<DataNode>`.
#[test]
fn contains_on_an_undeclared_collection_field_is_classified_via_the_ir() {
    let type_defs = table_ir();
    let resolver = ir_anchored_resolver(&type_defs, "ProcessResult");

    let out = render_contains(&resolver, "children", "Widget");

    assert!(
        out.contains(".iter().any("),
        "an undeclared collection field must render the per-element scan, got:\n{out}"
    );
    assert!(
        !out.contains(r##"result.children.contains(r#"Widget"#)"##),
        "must not render the scalar .contains(&str) shape, which would not compile against \
         Vec<DataNode>, got:\n{out}"
    );
}

/// A plain `String` field with the same name on an unrelated type must not be misclassified as
/// a collection — the IR classification is anchored per-call, not matched on the leaf name
/// alone.
#[test]
fn a_same_named_string_field_on_an_unrelated_type_is_not_misclassified_as_a_collection() {
    let type_defs = vec![TypeDef {
        name: "OtherResult".to_string(),
        fields: vec![children_field(TypeRef::String)],
        ..TypeDef::default()
    }];
    let resolver = ir_anchored_resolver(&type_defs, "OtherResult");

    let out = render_contains(&resolver, "children", "Widget");

    assert!(
        out.contains(r##"result.children.contains(r#"Widget"#)"##),
        "a plain string field's contains must keep using the scalar .contains(&str) shape, got:\n{out}"
    );
}
