//! Shared test fixture for the "wildcard element half must not be re-anchored to the result"
//! regression, used by every backend that expands a `container[].field` fixture path into an
//! any-element quantifier.
//!
//! ~keep The IR shape below is the whole point of the fixture and must not be simplified: the
//! defect only appears when the element leaf is UNDECLARED on the call's root type while a
//! `result_fields` entry reaches a type that does declare it. That is what makes
//! `FieldResolver::result_relative_path`'s envelope rescue fire, turning the element half `kind`
//! back into `records[0].kind` — which, rendered against a binding that is already an element,
//! re-applies the container path and addresses a member the element type has no such field for.
//! A flatter fixture (leaf declared on the root) produces a passing accessor either way and
//! would be a test that cannot fail.
//!
//! ~keep Nine backends need this identical shape, so it is built once here rather than copied
//! per backend: a drifted copy would silently stop reproducing the rescue and each backend's
//! regression test would go vacuously green.

use std::collections::{HashMap, HashSet};

use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// The IR type name the fixture resolver anchors its result-field oracle at.
pub(crate) const ROOT_TYPE: &str = "Report";

/// The IR type name the envelope resolver anchors at.
pub(crate) const ENVELOPE_ROOT_TYPE: &str = "Envelope";

/// The fixture path every backend test renders: `records` is a collection on the root,
/// `kind` is declared only on the element type.
pub(crate) const WILDCARD_FIELD: &str = "records[].kind";

/// A second fixture path whose leaf is likewise element-only — used by the `not_empty` arms,
/// which several backends build through a separate code path from `contains`.
pub(crate) const WILDCARD_NAME_FIELD: &str = "records[].name";

/// `Report { records: Vec<Entry> }`, `Entry { kind, name }` — the minimal shape that leaves the
/// element leaf undeclared on the root while reachable through a `result_fields` container.
fn report_entry_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: ROOT_TYPE.to_string(),
            fields: vec![FieldDef {
                name: "records".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Entry".to_string(),
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

/// A resolver wired exactly the way every backend's `call_field_resolver` wires one: IR field
/// facts and collection facts anchored at the call's declared result type, plus the
/// `result_fields` projection entry that the envelope rescue searches.
pub(crate) fn report_resolver(language: &str) -> FieldResolver {
    let type_defs = report_entry_type_defs();
    let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, language);
    let collection_map = FieldResolver::ir_collection_fields(&type_defs);
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let result_fields = HashSet::from(["records".to_string()]);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(result_field_map, Some(ROOT_TYPE.to_string()))
    .with_ir_collection_map(collection_map, Some(ROOT_TYPE.to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

/// `Envelope { results: Vec<Report> }` on top of the shape above — the container `records` is
/// now reachable only THROUGH a `result_fields` projection, so the container half genuinely
/// depends on the result anchoring `element_accessor` must not apply.
fn envelope_report_entry_type_defs() -> Vec<TypeDef> {
    let mut type_defs = vec![TypeDef {
        name: ENVELOPE_ROOT_TYPE.to_string(),
        fields: vec![FieldDef {
            name: "results".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named(ROOT_TYPE.to_string()))),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    type_defs.extend(report_entry_type_defs());
    type_defs
}

/// A resolver whose root is an envelope: `records` is NOT declared on the root, so the container
/// half of `records[].kind` only renders correctly when the result anchoring is applied.
///
/// ~keep This is the control the element-relative fix must not break. The element leaf `kind` is
/// unreachable through the `results` projection here (`Report` does not declare it), so the
/// envelope rescue declines for the element half and this fixture isolates the container half's
/// anchoring on its own: a change that simply stopped anchoring everywhere renders the container
/// as `records` off the root, which is a member the envelope does not declare.
pub(crate) fn envelope_resolver(language: &str) -> FieldResolver {
    let type_defs = envelope_report_entry_type_defs();
    let result_field_map = FieldResolver::ir_result_field_facts(&type_defs, language);
    let collection_map = FieldResolver::ir_collection_fields(&type_defs);
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let result_fields = HashSet::from(["results".to_string()]);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(result_field_map, Some(ENVELOPE_ROOT_TYPE.to_string()))
    .with_ir_collection_map(collection_map, Some(ENVELOPE_ROOT_TYPE.to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

/// Assert the container's rendered accessor fragment appears exactly once in `rendered`.
///
/// ~keep The sharpest statement of the defect that does not depend on a backend's lambda
/// parameter spelling: the container belongs to the iteration expression and nowhere else, so a
/// second occurrence is the container path re-applied inside the closure body. Takes the RENDERED
/// fragment (`.records()`, `->records`, `.Records`) rather than the bare fixture name, because
/// several backends also interpolate the raw fixture path into the failure message.
pub(crate) fn assert_container_accessor_appears_once(rendered: &str, container_accessor: &str) {
    let occurrences = rendered.matches(container_accessor).count();
    assert_eq!(
        occurrences, 1,
        "`{container_accessor}` must appear once (the iteration expression) and not a second time \
         inside the closure body, got {occurrences} in: {rendered}"
    );
}

/// A `contains` assertion on `field` looking for `"Heading"`.
pub(crate) fn contains_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!("Heading")),
        ..Default::default()
    }
}

/// A `not_empty` assertion on `field`.
pub(crate) fn not_empty_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    }
}

/// Assert that `rendered` carries an element-relative closure body and no re-applied container.
///
/// `element_body` is the accessor the backend must emit against its own element binding
/// (e.g. `e.Kind` for Go); `container_reapplied` is the fragment the pre-fix envelope rescue
/// produced against that same binding (e.g. `e.Records[0].Kind`).
pub(crate) fn assert_element_relative(rendered: &str, element_body: &str, container_reapplied: &str) {
    assert!(
        !rendered.contains(container_reapplied),
        "closure body must not re-apply the container path to the element binding \
         (`{container_reapplied}` addresses a member the element type does not declare), got: {rendered}"
    );
    assert!(
        rendered.contains(element_body),
        "closure body must address the element binding's own field (`{element_body}`), got: {rendered}"
    );
}
