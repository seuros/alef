//! `FieldResolver::is_array` must classify the path the emitters actually address.
//!
//! `accessor()` (and `result_relative_path()`, which the zig/brew/C generators navigate the
//! serialized result with) strip a virtual namespace prefix: a fixture field spelled
//! `interaction.action_results` addresses `action_results` on the result. `is_array` was a bare
//! `array_fields.contains(field)` lookup against the *unstripped* spelling, so the same path the
//! accessor had already reduced to a slice was classified as "not an array". `is_optional`, two
//! methods above it in the same file, has carried the namespace fallback since the feature
//! landed — the asymmetry is the bug, not a policy.

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn set(entries: &[&str]) -> HashSet<String> {
    entries.iter().map(|s| (*s).to_string()).collect()
}

/// The exact shape the bug produces: `action_results` is a declared result field and a declared
/// array field, and the fixture groups the assertion under a virtual `interaction.` label.
#[test]
fn is_array_strips_virtual_namespace_prefix() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["action_results", "final_url"]),
        &set(&["action_results"]),
        &HashSet::new(),
    );
    assert!(
        resolver.is_array("action_results"),
        "control: the unprefixed spelling was always classified as an array"
    );
    assert!(
        resolver.is_array("interaction.action_results"),
        "`interaction.` is a virtual label — the value it addresses is the same slice"
    );
}

/// The classification must agree with where `result_relative_path` says the value sits: those two
/// answers disagreeing is precisely what emitted a scalar conversion against a slice.
#[test]
fn is_array_agrees_with_result_relative_path() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["action_results"]),
        &set(&["action_results"]),
        &HashSet::new(),
    );
    let relative = resolver.result_relative_path("interaction.action_results");
    assert_eq!(relative, "action_results", "the virtual prefix must be stripped");
    assert!(
        resolver.is_array(relative),
        "the relative spelling names a declared array field"
    );
    assert!(
        resolver.is_array("interaction.action_results"),
        "the prefixed and relative spellings name one value and must classify identically"
    );
}

/// Negative control — a genuinely nested path keeps its prefix, so a leaf that happens to share a
/// name with a declared array field must NOT be promoted. Without this, a fix that stripped
/// unconditionally would pass the test above while breaking every real nested field.
#[test]
fn is_array_does_not_strip_a_real_nested_struct_field() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["metrics", "final_url"]),
        &set(&["total_lines"]),
        &HashSet::new(),
    );
    assert!(
        !resolver.is_array("metrics.total_lines"),
        "`metrics` is a declared result field, so it is a real struct step, not a virtual label"
    );
}

/// Negative control — stripping is opt-in on `result_fields` being configured, exactly as
/// `namespace_stripped_path` documents. With no `result_fields` there is no way to tell a virtual
/// label from a real nested step, so nothing may be stripped.
#[test]
fn is_array_does_not_strip_when_result_fields_is_unconfigured() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &set(&["action_results"]),
        &HashSet::new(),
    );
    assert!(
        !resolver.is_array("interaction.action_results"),
        "without result_fields the prefix is indistinguishable from a real nested field"
    );
}

/// Negative control — a prefix whose remainder is not a declared array field stays unclassified.
/// A fix that answered `true` for anything with a strippable prefix would pass the positive test
/// and be useless.
#[test]
fn is_array_still_rejects_a_stripped_path_that_is_not_an_array() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["final_url", "action_results"]),
        &set(&["action_results"]),
        &HashSet::new(),
    );
    assert!(
        !resolver.is_array("interaction.final_url"),
        "final_url is a declared result field but not an array"
    );
}

/// `result_relative_path` asks `is_valid_for_result`, which re-enters `is_array` through
/// `is_known_via_sibling_field_config`. This is the config shape that takes that route — the
/// stripped root is absent from `result_fields` and known only via `fields_array`. Reaching the
/// assertion at all is the proof the recursion terminates; a self-feeding fallback would blow the
/// stack here rather than return.
#[test]
fn is_array_terminates_when_the_stripped_root_is_known_only_through_fields_array() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["content"]),
        &set(&["items"]),
        &HashSet::new(),
    );
    assert!(
        resolver.is_array("ns.items"),
        "items is a declared array field, so the virtual `ns.` label must strip"
    );
}

/// An alias target that is a declared array field must classify through the raw fixture spelling
/// too. Most call sites already spell this `is_array(f) || is_array(resolve(f))`; routing through
/// `result_relative_path` (which resolves first) makes the two spellings agree everywhere.
#[test]
fn is_array_resolves_an_alias_to_its_array_target() {
    let mut aliases = HashMap::new();
    aliases.insert("results".to_string(), "action_results".to_string());
    let resolver = FieldResolver::new(
        &aliases,
        &HashSet::new(),
        &set(&["action_results"]),
        &set(&["action_results"]),
        &HashSet::new(),
    );
    assert!(
        resolver.is_array("results"),
        "the alias names the same slice as its target"
    );
}
