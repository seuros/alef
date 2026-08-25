//! An envelope-shaped result root must not turn a real nested struct hop into a virtual label.
//!
//! `namespace_stripped_path` strips a leading segment as a virtual namespace label when neither
//! `result_fields` nor `ir_declares_struct_field_on_root` claims it. That IR check only ever
//! inspects the root's OWN fields, so on an envelope root — one whose real payload sits behind a
//! `result_fields`-declared projection like `results: Vec<Document>` — a genuinely nested
//! `metadata.output_format` is indistinguishable from a virtual label and the `metadata` hop is
//! dropped entirely, addressing a member the root does not declare.
//!
//! ~keep The rescue must come from [`FieldResolver::anchor_leaf`], not a second copy of the prefix
//! search: the synthetic (`chunks`) path already asks it where an envelope-rooted leaf lives, and
//! two components deriving one fact independently is the defect shape this repo keeps re-growing.
//! The negative controls below pin the sanctioned behaviours that must survive the rescue.

use super::FieldResolver;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

fn set(entries: &[&str]) -> HashSet<String> {
    entries.iter().map(|s| (*s).to_string()).collect()
}

fn scalar_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..FieldDef::default()
    }
}

fn struct_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// `Envelope { results: Vec<Document> }`, `Document { metadata: Metadata }`,
/// `Metadata { output_format: String }` — the same shape `leaf_anchor`'s own tests use.
fn envelope_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![struct_field(
                "results",
                TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![struct_field("metadata", TypeRef::Named("Metadata".to_string()))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Metadata".to_string(),
            fields: vec![scalar_field("output_format")],
            ..TypeDef::default()
        },
    ]
}

/// A root that declares the nested struct directly — `Report { metrics: Metrics }` — so a real
/// nested path on a NON-envelope root can be pinned against the same wiring.
fn flat_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![
                struct_field("metrics", TypeRef::Named("Metrics".to_string())),
                scalar_field("final_url"),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Metrics".to_string(),
            fields: vec![scalar_field("total_lines")],
            ..TypeDef::default()
        },
    ]
}

/// The production wiring `rust/test_file/test_function.rs` builds: the result-field map and the
/// collection map anchored at the same resolved root, plus the consumer's `result_fields`.
fn resolver_for(type_defs: &[TypeDef], root: &str, result_fields: &[&str]) -> FieldResolver {
    let result_field_map = FieldResolver::ir_result_field_facts(type_defs, "rust");
    let collection_map = FieldResolver::ir_collection_fields(type_defs);
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(type_defs);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(result_fields),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(result_field_map, Some(root.to_string()))
    .with_ir_collection_map(collection_map, Some(root.to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

/// The defect: `metadata` is a real `Document` field reached through the `results` projection
/// `result_fields` names, but the root's own field list lacks it, so it read as a virtual label
/// and the whole hop vanished — `result.output_format`, a member `Envelope` does not declare.
#[test]
fn envelope_rooted_nested_path_keeps_every_segment() {
    let resolver = resolver_for(&envelope_type_defs(), "Envelope", &["results"]);
    assert_eq!(
        resolver.result_relative_path("metadata.output_format"),
        "results[0].metadata.output_format",
        "the shared answer must place the value behind the envelope projection, hop intact"
    );
    assert_eq!(
        resolver.accessor("metadata.output_format", "rust", "result"),
        "result.results[0].metadata.output_format"
    );
}

/// The same path spelled in full by the fixture author must land in exactly the same place — the
/// rescue must not double-prefix a path that already carries the projection.
#[test]
fn an_already_projected_path_is_not_prefixed_twice() {
    let resolver = resolver_for(&envelope_type_defs(), "Envelope", &["results"]);
    assert_eq!(
        resolver.accessor("results[0].metadata.output_format", "rust", "result"),
        "result.results[0].metadata.output_format"
    );
}

/// Negative control 1 — a genuinely virtual grouping label must still be stripped. `browser` is
/// no field of any type here, while `browser_used` is a real member of the root.
#[test]
fn a_genuinely_virtual_label_is_still_stripped() {
    let mut type_defs = envelope_type_defs();
    type_defs[0].fields.push(scalar_field("browser_used"));
    let resolver = resolver_for(&type_defs, "Envelope", &["results", "browser_used"]);
    assert_eq!(
        resolver.accessor("browser.browser_used", "python", "result"),
        "result.browser_used",
        "a label no type declares must still lose its segment"
    );
}

/// Negative control 2 — a real nested path on a NON-envelope root keeps its prefix and gains
/// nothing. The IR declares `metrics` on the root itself, so there is no projection to add.
#[test]
fn a_real_nested_path_on_a_non_envelope_root_keeps_its_prefix() {
    let resolver = resolver_for(&flat_type_defs(), "Report", &["final_url"]);
    assert_eq!(
        resolver.result_relative_path("metrics.total_lines"),
        "metrics.total_lines"
    );
    assert_eq!(
        resolver.accessor("metrics.total_lines", "python", "result"),
        "result.metrics.total_lines"
    );
}

/// Negative control 3 — an empty `result_fields` disables stripping entirely, and must equally
/// leave the envelope rescue inert: with nothing declared there is no candidate projection.
#[test]
fn an_empty_result_fields_still_disables_both_strip_and_prefix() {
    let resolver = resolver_for(&envelope_type_defs(), "Envelope", &[]);
    assert_eq!(
        resolver.result_relative_path("metadata.output_format"),
        "metadata.output_format"
    );
    assert_eq!(
        resolver.accessor("metadata.output_format", "python", "result"),
        "result.metadata.output_format"
    );
}

/// Negative control 4 — a first segment listed in `result_fields` is never a namespace prefix,
/// and must not be re-anchored either: `results` IS the projection, it cannot sit behind itself.
#[test]
fn a_first_segment_listed_in_result_fields_is_never_treated_as_a_prefix() {
    let resolver = resolver_for(&envelope_type_defs(), "Envelope", &["results"]);
    assert_eq!(resolver.result_relative_path("results"), "results");
    assert_eq!(resolver.accessor("results", "python", "result"), "result.results");
}

/// Negative control 5 — a path no projection reaches must stay exactly where it is today. The
/// rescue is additive: it may only relocate a path the IR positively declares behind a projection.
#[test]
fn a_path_no_projection_reaches_is_left_alone() {
    let resolver = resolver_for(&envelope_type_defs(), "Envelope", &["results"]);
    assert_eq!(
        resolver.result_relative_path("not_a_type.not_a_field"),
        "not_a_type.not_a_field",
        "unchanged from today: the strip is refused because the remainder is not a known member"
    );
}

/// Config-only wiring (no IR anchored at all) must be untouched by the rescue — every fixture
/// suite without a resolved result type is in this state, so a prefix invented from
/// `result_fields` alone would relocate paths across the whole generator.
#[test]
fn a_config_only_resolver_is_never_prefixed() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["results"]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(
        resolver.result_relative_path("metadata.output_format"),
        "metadata.output_format",
        "unchanged from today: with no IR, nothing confirms `output_format` is where the value sits"
    );
}
