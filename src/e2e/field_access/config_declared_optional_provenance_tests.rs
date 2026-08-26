//! Regression coverage for [`FieldResolver::declaring_config_key`] telling "this field
//! classifies as optional" apart from "the consumer's own `[e2e].fields_optional` named it" --
//! see `config_declared_optional_fields`'s field doc in `types.rs` for why the two answers have
//! to come from separate sets.
//!
//! A downstream consumer's regen emitted warnings naming `fields_optional` for two field names
//! that never appeared anywhere in their `alef.toml`. `with_ir_fields`'s deliberate merge of
//! IR-derived `Option<T>` names into `optional_fields` had made `declaring_config_key` treat an
//! IR-only name as if the consumer had written it down, and the diagnostic told them to correct
//! or delete a config entry that was never there.

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn resolver_with(config_optional: &[&str], ir_optional: &[&str]) -> FieldResolver {
    let optional: HashSet<String> = config_optional.iter().map(|s| s.to_string()).collect();
    let ir_optional: HashSet<String> = ir_optional.iter().map(|s| s.to_string()).collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(HashSet::new(), HashSet::new(), ir_optional)
}

/// The regression: a name the IR alone proved `Option<T>` -- nothing in `[e2e].fields_optional`
/// named it -- must not be reported as config-declared. Classification must stay unaffected: the
/// IR-derived name still guards as optional. Only the PROVENANCE claim was wrong before the fix.
#[test]
fn an_ir_only_optional_name_is_not_reported_as_config_declared() {
    let resolver = resolver_with(&[], &["structured_output"]);

    assert!(
        resolver.is_optional("structured_output"),
        "classification must stay permissive: the IR really does prove this field optional"
    );
    assert_eq!(
        resolver.declaring_config_key("structured_output"),
        None,
        "nothing in `[e2e].fields_optional` named this field -- the diagnostic must not claim it did"
    );
}

/// The negative control: a name the consumer genuinely listed under `fields_optional` must still
/// be reported with that key. Proves the provenance check can still fire -- a version of
/// `declaring_config_key` that always answered `None` for `fields_optional` would pass the test
/// above for the wrong reason, and this is what catches that.
#[test]
fn a_config_declared_optional_name_is_still_reported_with_its_key() {
    let resolver = resolver_with(&["legacy_field"], &[]);

    assert!(resolver.is_optional("legacy_field"));
    assert_eq!(resolver.declaring_config_key("legacy_field"), Some("fields_optional"));
}

/// Both together, in the same resolver: a config-declared entry keeps its provenance once the IR
/// merge has also run, and the IR-only name sitting right next to it still doesn't gain a false
/// one.
#[test]
fn config_declared_and_ir_only_names_are_told_apart_in_the_same_resolver() {
    let resolver = resolver_with(&["legacy_field"], &["structured_output"]);

    assert!(resolver.is_optional("legacy_field"));
    assert!(resolver.is_optional("structured_output"));
    assert_eq!(resolver.declaring_config_key("legacy_field"), Some("fields_optional"));
    assert_eq!(resolver.declaring_config_key("structured_output"), None);
}
