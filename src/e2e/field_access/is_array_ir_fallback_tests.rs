//! `FieldResolver::is_array` must fall back to the IR-anchored answer
//! (`ir_collection::is_collection_path`) when `fields_array` never named the field, mirroring
//! `is_optional`'s existing IR fallback (`ir_result_fields::is_optional_path`).
//!
//! ~keep Before this fix, `is_array` consulted ONLY the hand-maintained `fields_array` config set
//! (plus `result_relative_path`'s namespace-stripped spelling of it) — never the IR. A field whose
//! `Vec`-ness is known SOLELY through the crate's own IR (no per-element path anywhere in the
//! fixture suite ever populated `fields_array`) read as scalar even while `is_optional` correctly
//! read it as optional via its own IR fallback. Every caller that branches on
//! `is_optional(..) && is_array(..)` to decide whether an `Option<Vec<T>>` needs
//! `.as_ref().is_some_and(|v| ...)` rather than a bare method call took the field for an "optional
//! scalar" instead, and emitted the bare call directly against the still-wrapped `Option`. This is
//! the exact shape the reported CI failure took: `render_count_min_assertion`
//! (`rust/assertion_helpers.rs`) emitted `result.results[0].chunks.len() >= 2` against
//! `Option<Vec<Chunk>>` because `is_array("results[0].chunks")` answered `false` with no
//! `fields_array` entry naming it, even though `is_optional` correctly answered `true`.

use super::FieldResolver;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn set(entries: &[&str]) -> HashSet<String> {
    entries.iter().map(|s| (*s).to_string()).collect()
}

/// `Document { chunks: Option<Vec<Chunk>>, title: Option<String> }` — `chunks` is the collection
/// under test, `title` is a same-shape (`Option<..>`) but non-collection sibling field so a fix
/// that treats "optional" as "array" by accident cannot pass by coincidence.
fn document_type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Document".to_string(),
        fields: vec![
            FieldDef {
                name: "chunks".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                optional: true,
                ..FieldDef::default()
            },
            FieldDef {
                name: "title".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    }]
}

fn resolver_anchored_at_document() -> FieldResolver {
    let type_defs = document_type_defs();
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_collection_map(
        FieldResolver::ir_collection_fields(&type_defs),
        Some("Document".to_string()),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts(&type_defs, "rust"),
        Some("Document".to_string()),
    )
    .with_ir_fields(reachable, excluded, optional)
}

/// The confirmed defect: `chunks` is a `Vec<T>` field known only through the IR — `fields_array`
/// is empty — and `is_array` must still answer `true`.
///
/// Revert symptom: `is_array` returns to consulting ONLY `array_fields`/`result_relative_path`, so
/// this fails with "is_array must be true when only the IR knows chunks is a Vec, got: false".
#[test]
fn is_array_true_from_ir_when_fields_array_config_is_silent() {
    let resolver = resolver_anchored_at_document();
    assert!(
        resolver.is_array("chunks"),
        "is_array must be true when only the IR knows chunks is a Vec, got: false"
    );
}

/// Negative control on the SAME owning type: `title` is `Option<String>`, not a collection, so a
/// fix that conflates "optional" with "array" (e.g. by reusing `is_optional`'s answer) must not
/// make this true.
///
/// Revert symptom: none directly (this passes both before and after a correct fix) — it exists to
/// catch an OVER-broad fix, and would fail ("title must not be classified as an array") if a
/// future change made `is_array` answer `true` for every optional field regardless of shape.
#[test]
fn is_array_stays_false_for_an_optional_non_collection_sibling_field() {
    let resolver = resolver_anchored_at_document();
    assert!(
        !resolver.is_array("title"),
        "title must not be classified as an array, got: true"
    );
}

/// The production shape the CI failure actually took: `Envelope { results: Vec<Document> }`,
/// reached at `results[0].chunks`, with NO `fields_array` config entry anywhere. Mirrors
/// `rust/assertions/chunks_anchoring_tests.rs`'s envelope shape.
///
/// Revert symptom: fails with "is_array must be true for the full projected path
/// results[0].chunks, got: false" — the same path `render_count_min_assertion` resolves for the
/// reported `result.results[0].chunks.len() >= 2` compile failure.
#[test]
fn is_array_true_for_full_envelope_projected_path() {
    let type_defs = vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![FieldDef {
                name: "results".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![FieldDef {
                name: "chunks".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ];
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let result_fields = set(&["results"]);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_collection_map(
        FieldResolver::ir_collection_fields(&type_defs),
        Some("Envelope".to_string()),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts(&type_defs, "rust"),
        Some("Envelope".to_string()),
    )
    .with_ir_fields(reachable, excluded, optional);
    assert!(
        resolver.is_array("results[0].chunks"),
        "is_array must be true for the full projected path results[0].chunks, got: false"
    );
    // Sanity: `is_optional` already answered this correctly before the fix — pinning it here
    // proves the two oracles now agree, which is the whole point of the fix.
    assert!(
        resolver.is_optional("results[0].chunks"),
        "sanity: is_optional was already true for this path before the fix"
    );
}

/// Negative control — no anchored root type at all (every config-only resolver, and the state of
/// every call site before IR wiring existed) must keep the permissive-but-unhelpful pre-existing
/// default: `false`, not a panic or a spurious `true`.
///
/// Revert symptom: none expected from reverting the fix (both old and new code return `false`
/// here) — this guards against a DIFFERENT bad fix that assumes a root is always present.
#[test]
fn is_array_stays_false_with_no_anchored_root_type() {
    let type_defs = document_type_defs();
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_collection_map(FieldResolver::ir_collection_fields(&type_defs), None)
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(&type_defs, "rust"), None)
    .with_ir_fields(reachable, excluded, optional);
    assert!(
        !resolver.is_array("chunks"),
        "no anchored root type means no IR answer; must stay false, got: true"
    );
}

/// Config still wins when it says so, even without any IR wired in at all — additivity: the fix
/// must never SUBTRACT a `true` answer the config already gave.
///
/// Revert symptom: none (this passes on both sides) — regresses only if a future change makes the
/// IR fallback somehow override or short-circuit before the config-declared check.
#[test]
fn is_array_true_from_config_alone_with_no_ir_wired_in() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &set(&["chunks"]),
        &HashSet::new(),
    );
    assert!(
        resolver.is_array("chunks"),
        "config-declared array field must stay true"
    );
}
