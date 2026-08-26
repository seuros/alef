//! `is_valid_for_result` must stay permissive for a name declared only on an unrelated IR
//! struct, while `result_field_oracle_knows` -- the oracle a DERIVED docs-snippet accessor is
//! judged against -- must refuse it. Split out of `resolver::classify`'s own test module to keep
//! that file under the repo's line-count cap; the tests are unchanged.
//!
//! ~keep A prior attempt (unmerged, `bc67f7fc1`) anchored `is_valid_for_result` the same way
//! `result_field_oracle_knows` is anchored, on the theory that a name declared on ANY IR struct
//! (not the call's own result type) validating a fixture field was, categorically, the same
//! defect the `chunks`/`document_structure` production incidents already fixed via
//! `result_field_oracle_knows`. It is not: `presentation.rs::shows_on_result` already consults
//! BOTH oracles for the DERIVED docs-snippet path -- `is_valid_for_result` rejects only what is
//! positively `ir_known_excluded_fields`, `result_field_oracle_knows` ALSO rejects what it has
//! simply never anchored-confirmed -- and
//! `e2e::codegen::presentation::anchored_result_facts_tests`/`deep_result_path_tests` pin, with
//! real incident citations, that `is_valid_for_result` staying permissive for a HAND-AUTHORED
//! fixture assertion is deliberate, not the bug: a config-declared or IR-elsewhere-reachable name
//! must keep rendering an assertion even when the call's own anchored result type does not
//! declare it, precisely so a real, working assertion is never silently dropped. Anchoring
//! `is_valid_for_result` the way `bc67f7fc1` did makes it agree with `result_field_oracle_knows`
//! in exactly the cases those two test modules require them to DISAGREE, which is a regression,
//! not a fix. This module exists so that regression is caught here, at the unit level, without
//! needing the full presentation-layer suite.

use super::FieldResolver;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// `Envelope { results: Vec<Document> }`, `Document { chunks: Vec<Chunk> }`. `chunks` is
/// declared on `Document`, reached only through `Envelope.results`, never directly on
/// `Envelope`.
fn envelope_and_document_type_defs() -> Vec<TypeDef> {
    vec![
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
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn resolver_anchored_at(root_type: &str) -> FieldResolver {
    let type_defs = envelope_and_document_type_defs();
    let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(map, Some(root_type.to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

/// The asymmetry, at the root: `chunks` belongs to `Document`, not the call's own anchored
/// root `Envelope`. `is_valid_for_result` must still validate it (a hand-authored assertion
/// on it must still render), while `result_field_oracle_knows` — the oracle a DERIVED
/// accessor is judged against — must refuse it.
#[test]
fn is_valid_for_result_stays_permissive_for_a_name_declared_only_on_an_unrelated_ir_struct() {
    let resolver = resolver_anchored_at("Envelope");
    assert!(
        resolver.is_valid_for_result("chunks"),
        "a hand-authored assertion on `chunks` must still render even though `Envelope` \
         does not declare it"
    );
    assert_eq!(
        resolver.result_field_oracle_knows("chunks"),
        Some(false),
        "a derived accessor for `chunks` must be refused: `Envelope` does not declare it"
    );
}

/// The control: when the call's root type genuinely declares the field, both oracles must
/// agree it is valid — the asymmetry above is specific to the anchor's `Some(false)` case,
/// not a blanket disagreement.
#[test]
fn both_oracles_agree_when_the_call_own_result_type_declares_the_field() {
    let resolver = resolver_anchored_at("Document");
    assert!(resolver.is_valid_for_result("chunks"));
    assert_eq!(resolver.result_field_oracle_knows("chunks"), Some(true));
}

/// No resolved root type at all (every call site before result_type anchoring existed, and
/// every call site today that still can't resolve one) leaves `result_field_oracle_knows`
/// with nothing anchored to answer from, so it falls back to the same flat, name-keyed
/// check `is_valid_for_result` always used — the two agree here too, just not through the
/// anchor.
#[test]
fn unresolved_root_type_leaves_both_oracles_on_the_flat_fallback() {
    let type_defs = envelope_and_document_type_defs();
    let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(map, None)
    .with_ir_fields(reachable, excluded, optional);

    assert!(resolver.is_valid_for_result("chunks"));
    assert_eq!(resolver.result_field_oracle_knows("chunks"), Some(true));
}
