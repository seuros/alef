//! `FieldResolver::accessor` must address the path [`FieldResolver::result_relative_path`] names.
//!
//! Where a fixture field's value sits had three implementations: `result_relative_path` (the zig,
//! brew and C generators' shared answer, and — since `is_array` was aligned onto it — the
//! classifiers' too), and two private copies inside `accessor()` and `rust_unwrap_binding()` that
//! re-derived the same decision from a narrower predicate, `result_fields.contains(..)` instead of
//! `is_valid_for_result(..)`.
//!
//! The narrow copy was not a deliberate policy, it was the un-updated original: it predates the IR
//! oracle that `is_valid_for_result` grew, and the two C call sites that inline the very same block
//! (`c/call_patterns.rs`, `c/test_function.rs`) comment it "matching the same logic as
//! FieldResolver::accessor" while using the *broad* predicate. `result_relative_path`'s own doc
//! likewise asserts it is "the same policy `accessor()` applies". Three places claimed an equality
//! that did not hold.
//!
//! ~keep These tests pin the equality itself, not one worked example of it: an accessor emitted
//! against a path no classifier agrees is where the value lives is the shape that shipped
//! `string(result.ActionResults)` into a generated Go package.

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn set(entries: &[&str]) -> HashSet<String> {
    entries.iter().map(|s| (*s).to_string()).collect()
}

/// `result_fields` is hand-maintained and under-reports; the IR does not. A consumer who listed
/// some other field, so stripping is enabled at all, but never listed `action_results`, still has
/// an IR that reaches it — and `is_valid_for_result` treats that as the primary source of truth.
/// The accessor must follow, or it emits a member access against a virtual label.
#[test]
fn accessor_strips_a_prefix_the_ir_recognizes_but_result_fields_omits() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(set(&["action_results"]), HashSet::new(), HashSet::new());

    assert_eq!(
        resolver.result_relative_path("interaction.action_results"),
        "action_results",
        "control: the shared answer strips the virtual label on the IR's word"
    );
    assert_eq!(
        resolver.accessor("interaction.action_results", "python", "result"),
        "result.action_results",
        "the accessor must address the same place the shared answer names"
    );
}

/// The `is_known_via_sibling_field_config` route: the stripped root is absent from `result_fields`
/// and known only because the consumer configured `fields_array` against it. Configuring a field
/// is evidence the config author looked at the real struct, so `is_valid_for_result` accepts it —
/// and the accessor, being what `is_array`'s classification is compared against, has to agree.
#[test]
fn accessor_strips_a_prefix_known_only_through_fields_array() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["final_url"]),
        &set(&["action_results"]),
        &HashSet::new(),
    );

    assert!(
        resolver.is_array("interaction.action_results"),
        "control: the classifier calls this a slice"
    );
    assert_eq!(
        resolver.accessor("interaction.action_results", "go", "result"),
        "result.ActionResults",
        "a path the classifier calls a slice must be emitted as that slice, not as `result.Interaction...`"
    );
}

/// The already-working case, kept as a control: when `result_fields` does list the stripped root,
/// the narrow and broad predicates agree and the emitted accessor must not move.
#[test]
fn accessor_still_strips_a_prefix_result_fields_declares() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["action_results", "final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(
        resolver.accessor("interaction.action_results", "python", "result"),
        "result.action_results"
    );
}

/// Negative control — a genuinely nested struct path keeps its prefix. Without this, widening the
/// predicate to "always strip" would pass every positive test above and break real nested access.
#[test]
fn accessor_does_not_strip_a_real_nested_struct_field() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["metrics", "final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(
        resolver.result_relative_path("metrics.total_lines"),
        "metrics.total_lines",
        "control: `metrics` is a declared result field, so it is a real struct step"
    );
    assert_eq!(
        resolver.accessor("metrics.total_lines", "python", "result"),
        "result.metrics.total_lines"
    );
}

/// Negative control — stripping stays opt-in on `result_fields` being configured at all, exactly
/// as `namespace_stripped_path` documents. Routing through `result_relative_path` must not turn
/// the unconfigured case into a strip.
#[test]
fn accessor_does_not_strip_when_result_fields_is_unconfigured() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &set(&["action_results"]),
        &HashSet::new(),
    );
    assert_eq!(
        resolver.accessor("interaction.action_results", "python", "result"),
        "result.interaction.action_results",
        "with no result_fields a virtual label is indistinguishable from a real nested field"
    );
}

/// Negative control — a strippable prefix whose remainder no oracle recognizes must keep its
/// prefix. `is_valid_for_result` returns `true` for names it has never heard of, but that
/// default-allow is reached through the *first* segment; the stripped root here is judged against
/// a populated `result_fields`, so an unknown remainder is a real rejection.
#[test]
fn accessor_keeps_a_prefix_whose_remainder_no_oracle_recognizes() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(
        resolver.result_relative_path("interaction.unknown_thing"),
        "interaction.unknown_thing",
        "control: the shared answer declines to strip"
    );
    assert_eq!(
        resolver.accessor("interaction.unknown_thing", "python", "result"),
        "result.interaction.unknown_thing"
    );
}

/// `rust_unwrap_binding` carried a verbatim second copy of the same block, introduced to "mirror
/// the namespace-prefix stripping done in `accessor()`". It has to land on the same path too —
/// its generated local name is derived from that path, so a disagreement renames the binding an
/// assertion then fails to reference.
#[test]
fn rust_unwrap_binding_strips_the_same_prefix_the_accessor_does() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &set(&["action_results"]),
        &set(&["final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(set(&["action_results"]), HashSet::new(), HashSet::new());

    let (binding, local) = resolver
        .rust_unwrap_binding("interaction.action_results", "result")
        .expect("the field is declared optional, so a binding is generated");
    assert_eq!(local, "_action_results", "the local is named after the stripped path");
    assert!(
        binding.contains("result.action_results"),
        "expected the stripped accessor in the binding, got: {binding}"
    );
}

/// The invariant itself, over every spelling the resolver distinguishes: whatever
/// `result_relative_path` returns is what a plain dot-access accessor renders. Pinning the
/// relationship rather than a table of literals is what stops the two from drifting apart again —
/// a future edit to either side has to keep them equal or fail here.
#[test]
fn accessor_renders_exactly_the_result_relative_path_for_every_shape() {
    let mut aliases = HashMap::new();
    aliases.insert("results".to_string(), "action_results".to_string());
    let resolver = FieldResolver::new(
        &aliases,
        &HashSet::new(),
        &set(&["metrics", "final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(set(&["action_results"]), HashSet::new(), HashSet::new());

    for field in [
        "interaction.action_results",
        "metrics.total_lines",
        "interaction.unknown_thing",
        "final_url",
        "results",
    ] {
        let expected = format!("result.{}", resolver.result_relative_path(field));
        assert_eq!(
            resolver.accessor(field, "python", "result"),
            expected,
            "accessor and result_relative_path disagree about where `{field}` lives"
        );
    }
}

/// The one case where the shared answer is *narrower* than the copy it replaces, pinned so the
/// trade is deliberate. `is_valid_for_result` lets the IR override `result_fields` for a field the
/// IR marks `binding_excluded`, and `with_ir_fields` warns loudly that such an entry is a config
/// bug: no binding emits an accessor for it, so `result.action_results` would not compile either.
/// Neither spelling compiles here; what changes is that the accessor now lands where `is_array`,
/// and the zig/brew/C serialized-path navigation, say the value lives. Agreeing with them beats
/// keeping a private answer for a config state the resolver already reports as broken. ~keep
#[test]
fn accessor_declines_to_strip_onto_a_field_the_ir_marks_binding_excluded() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["action_results", "final_url"]),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(HashSet::new(), set(&["action_results"]), HashSet::new());

    assert_eq!(
        resolver.result_relative_path("interaction.action_results"),
        "interaction.action_results",
        "control: the IR overrides the contradicting result_fields entry"
    );
    assert_eq!(
        resolver.accessor("interaction.action_results", "python", "result"),
        "result.interaction.action_results",
        "the accessor must not keep a private answer for a config state the resolver rejects"
    );
}
