//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the Rust e2e generator.
//!
//! Split into its own file rather than added to `rust/assertions.rs`: that file sits exactly at
//! its recorded ceiling in `tests/file_size_baseline.txt`, so new coverage goes into a fresh
//! module instead of growing it (see `file-modularization` in CLAUDE.md). ~keep
//!
//! The defect: `render_rust_wildcard_assertion` splits `structure[].kind` into the container
//! `structure` and the element half `kind`, then built the closure body with
//! `FieldResolver::accessor`, which anchors a path against the call's RESULT type. `kind` is not
//! declared on the root, so `envelope_projected_path` "rescued" it through the `result_fields`
//! entry that does reach it — `structure` — and handed back `structure[0].kind`. Rendered against
//! the closure binding that is ALREADY an element, that is `e.structure[0].kind`: the container
//! path applied a second time, i.e. `E0609: no field 'structure'` on the element type. The
//! generator's own assertion MESSAGE was right throughout (it is built from the raw fixture path),
//! which is what made the mismatch visible. `element_accessor` anchors at the element instead.

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// `Report { structure: Vec<Element> }`, `Element { kind, name }` — the minimal shape that makes
/// the element leaf undeclared on the root while reachable through a `result_fields` container.
fn report_element_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![FieldDef {
                name: "structure".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Element".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Element".to_string(),
            fields: vec![
                FieldDef {
                    name: "kind".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "name".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
    ]
}

fn report_resolver() -> FieldResolver {
    let type_defs = report_element_type_defs();
    let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let collection_map = FieldResolver::ir_collection_fields(&type_defs);
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let result_fields = HashSet::from(["structure".to_string()]);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(result_field_map, Some("Report".to_string()))
    .with_ir_collection_map(collection_map, Some("Report".to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

fn render_wildcard(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> String {
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
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
        &report_resolver(),
        false,
        false,
        false,
        false,
        false,
        None,
    );
    out
}

/// The exact bug report, asserted on the emitted accessor expression rather than on "generation
/// succeeded": the closure body must address the binding's own field, not re-walk the container.
#[test]
fn wildcard_closure_body_is_relative_to_the_element_binding() {
    let rendered = render_wildcard("contains", "structure[].kind", Some(serde_json::json!("Function")));

    assert!(
        !rendered.contains("e.structure"),
        "closure body must not re-apply the container path to the element binding \
         (`e.structure[0].kind` is E0609: no field `structure` on the element type), got: {rendered}"
    );
    assert!(
        rendered.contains("|e| e.kind"),
        "closure body must address the element binding's own field (`|e| e.kind`), got: {rendered}"
    );
}

/// The container half must keep its result anchoring — the fix must not turn into "never anchor".
/// Without this, dropping anchoring wholesale would still pass the test above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render_wildcard("contains", "structure[].kind", Some(serde_json::json!("Function")));

    assert!(
        rendered.contains("result.structure.iter().any(|e|"),
        "container half must still resolve against the result variable, got: {rendered}"
    );
}

/// The message and the accessor are built from the same fixture path and must stay in agreement:
/// a message naming `structure[].kind` beside an accessor reaching into `structure[0]` is the
/// exact disagreement this defect presented as.
#[test]
fn wildcard_message_and_accessor_describe_the_same_path() {
    let rendered = render_wildcard("contains", "structure[].kind", Some(serde_json::json!("Function")));

    assert!(
        rendered.contains("expected some element of structure[].kind to contain"),
        "message must name the fixture path, got: {rendered}"
    );
    assert!(
        !rendered.contains("structure[0]"),
        "neither message nor accessor may index the container the wildcard already quantifies \
         over, got: {rendered}"
    );
}

/// `not_empty` builds its predicate through the same element accessor, so it regresses and
/// recovers with `contains` — a fix applied to one arm only would leave this red.
#[test]
fn wildcard_not_empty_predicate_is_also_element_relative() {
    let rendered = render_wildcard("not_empty", "structure[].name", None);

    assert!(
        !rendered.contains("e.structure"),
        "not_empty closure body must not re-apply the container path, got: {rendered}"
    );
    assert!(
        rendered.contains("|e| !e.name"),
        "not_empty closure body must address the element binding's own field, got: {rendered}"
    );
}

/// The emitted line must be real Rust, not just the right substring.
#[test]
fn wildcard_assertion_emits_parseable_rust() {
    let body = render_wildcard("contains", "structure[].kind", Some(serde_json::json!("Function")));
    let unit = format!("fn generated() {{\n{body}}}\n");
    syn::parse_file(&unit).unwrap_or_else(|error| panic!("must emit parseable Rust: {error}\n{unit}"));
}
