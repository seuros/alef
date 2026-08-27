//! Whether `warn_unmatched_exclude_entries` treats a `[crates.exclude]` entry that names a
//! generic public item differently from one that matches nothing at all -- split out of
//! [`super`] (`tests.rs`) alongside `config_entry_matching.rs`. Uses `super`'s private fixture
//! helpers (`make_typedef`, `make_funcdef`, `surface_with`).
//!
//! A private item and a cfg-gated-out item are never recorded anywhere in `ApiSurface` -- alef's
//! extraction only records an item once it has decided the item is public (see
//! `src/extract/extractor/mod.rs`'s `is_pub` guards). That makes them structurally identical, at
//! this layer, to a genuine typo: both are a name with zero footprint in the IR. Only a generic
//! public item is provable, because it is recorded as an `unsupported_public_items` diagnostic
//! before being dropped. These tests assert exactly that split: the provable case goes quiet, and
//! everything else -- including a name meant to stand in for a private item -- stays loud,
//! because staying loud is the only way a genuine typo is guaranteed to still warn. ~keep

use super::*;
use crate::core::config::ExcludeConfig;
use crate::core::ir::UnsupportedPublicItem;

fn generic_unsupported_item(kind: &str, name: &str) -> UnsupportedPublicItem {
    UnsupportedPublicItem {
        item_kind: kind.to_string(),
        item_path: format!("my_crate::{name}"),
        reason: format!("public generic {kind}s cannot be represented without explicit monomorphization metadata"),
        suggested_fix: "exclude the item, configure an opaque/bridge policy, or provide explicit \
                         monomorphization metadata"
            .to_string(),
    }
}

/// An `exclude.types` entry naming a public item alef recorded as generic (and therefore never
/// extracted) is provable from `unsupported_public_items`: it must be classified as redundant,
/// not reported as if it matched nothing.
#[test]
fn generic_public_item_is_redundant_not_unmatched() {
    let mut surface = surface_with(vec![make_typedef("Kept")], vec![]);
    surface.unsupported_public_items = vec![generic_unsupported_item("struct", "RetryConfig")];

    let exclude = ExcludeConfig {
        types: vec!["RetryConfig".to_string()],
        functions: vec![],
        methods: vec![],
        fields: vec![],
    };

    assert_eq!(
        redundant_generic_exclude_entries(&surface, &exclude),
        vec![("types", "RetryConfig".to_string())],
        "a known-generic entry must be classified as redundant"
    );
    assert!(
        unmatched_exclude_entries(&surface, &exclude).is_empty(),
        "a known-generic entry must not also be reported as unmatched"
    );
}

/// A private item is never recorded anywhere in `ApiSurface` -- extraction drops it before it
/// becomes a `TypeDef` or an `UnsupportedPublicItem`. An `exclude.types` entry naming one
/// therefore has the identical (empty) footprint a genuine typo would have, and MUST stay in the
/// loud `unmatched_exclude_entries` bucket: there is no cheap, safe way to tell "this name is
/// private" apart from "this name does not exist", and silently downgrading one would silently
/// downgrade the other.
#[test]
fn private_item_name_is_unmatched_not_redundant() {
    let surface = surface_with(vec![make_typedef("Kept")], vec![]);

    let exclude = ExcludeConfig {
        types: vec!["InternalRetryState".to_string()],
        functions: vec![],
        methods: vec![],
        fields: vec![],
    };

    assert!(
        redundant_generic_exclude_entries(&surface, &exclude).is_empty(),
        "a private item's name carries no generic diagnostic and must not be classified as redundant"
    );
    assert_eq!(
        unmatched_exclude_entries(&surface, &exclude),
        vec![("types", "InternalRetryState".to_string())],
        "a private item's name must still be reported as unmatched, exactly like a typo"
    );
}

/// The negative control: a genuinely wrong entry -- a typo naming nothing that exists in source
/// at all -- must still warn loudly. This is the one case the whole diagnostic exists to catch,
/// and it must never go quiet.
#[test]
fn genuine_typo_is_unmatched_not_redundant() {
    let surface = surface_with(vec![make_typedef("Kept")], vec![]);

    let exclude = ExcludeConfig {
        types: vec!["RetryConfg".to_string()],
        functions: vec!["cuont_tokens".to_string()],
        methods: vec!["Kept.walk".to_string()],
        fields: vec![],
    };

    assert!(
        redundant_generic_exclude_entries(&surface, &exclude).is_empty(),
        "a typo must never be classified as a redundant generic exclusion"
    );

    let mut unmatched = unmatched_exclude_entries(&surface, &exclude);
    unmatched.sort();
    assert_eq!(
        unmatched,
        vec![
            ("functions", "cuont_tokens".to_string()),
            ("methods", "Kept.walk".to_string()),
            ("types", "RetryConfg".to_string()),
        ],
        "every typo'd entry across all three lists must still warn"
    );
}

/// Only a `generic` diagnostic reason earns the quiet *redundant* classification. An
/// `unsupported_public_items` entry recorded for a different reason is still a provable, already-
/// matched diagnostic -- `unmatched_exclude_entries` treats any recorded diagnostic as matched,
/// regardless of reason, and that part is unchanged here -- so it must not be swept into
/// "redundant" (which is reserved for the specific, provable generic case) just because some
/// diagnostic exists.
#[test]
fn unsupported_item_with_non_generic_reason_is_not_classified_as_redundant() {
    let mut surface = surface_with(vec![make_typedef("Kept")], vec![]);
    surface.unsupported_public_items = vec![UnsupportedPublicItem {
        item_kind: "function".to_string(),
        item_path: "my_crate::do_thing".to_string(),
        reason: "public async trait methods are not yet representable".to_string(),
        suggested_fix: "exclude the item".to_string(),
    }];

    let exclude = ExcludeConfig {
        types: vec![],
        functions: vec!["do_thing".to_string()],
        methods: vec![],
        fields: vec![],
    };

    assert!(
        redundant_generic_exclude_entries(&surface, &exclude).is_empty(),
        "a non-generic unsupported-item reason must not be classified as redundant"
    );
    assert!(
        unmatched_exclude_entries(&surface, &exclude).is_empty(),
        "a recorded (if non-generic) diagnostic is still a match, so it must not warn either"
    );
}
