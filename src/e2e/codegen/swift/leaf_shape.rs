//! What shape the swift-bridge getter for an assertion's LEAF segment has, and which assertions
//! that shape makes unspellable.
//!
//! Split out of `assertions.rs` because these are one concern with one source of truth — the
//! binding's own getter classification, carried on `SwiftFirstClassMap` — consulted by several
//! unrelated arms of the assertion renderer. Keeping the verdicts here means `assertions.rs`
//! decides what to *emit* while this module decides what the leaf *is*.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;

/// Suffixes that ask for a collection's element count.
const COUNT_SUFFIXES: [&str; 3] = ["length", "count", "size"];

/// Render the skip line for a path that steps *past* a JSON-bridged leaf, if it does.
///
/// ~keep swift-bridge collapses a JSON-bridged field to one `RustString`, so the leaf has neither
/// `.count` nor a subscript, and every way of stepping past it is equally unspellable. The guard
/// this replaced was keyed on the trailing accessor's spelling, so it caught a count suffix and
/// missed an index or wildcard on the very same field — the generator wrote the correct
/// "JSON-bridges it to RustString" skip for one and a broken assertion for the other, on adjacent
/// lines. Deciding from the single fact that makes any of them impossible collapses four cases
/// into one.
pub(super) fn json_bridged_traversal_skip(field_resolver: &FieldResolver, field: Option<&str>) -> Option<String> {
    let field = field.filter(|f| !f.is_empty())?;
    let bridged = field_resolver.swift_json_bridged_traversal_prefix(field)?;
    Some(skip_line(&bridged))
}

/// Render the skip line for a count suffix whose collection leaf is not a countable `RustVec`.
///
/// ~keep Runs only after `is_valid_for_result` accepted the path, so the field IS resolvable, and
/// `NotAvailableOnResultType` — an `AuthoringGap`, therefore fatal under the strict gate — was the
/// wrong wording for it: the backend dropped the assertion as an honest ABI limit while the gate
/// demanded the consumer repair a field path that was never wrong, two verdicts about one fact
/// with nothing comparing them. `CountOnJsonBridgedLeafInSwift` states the real reason and carries
/// the classification that reason implies.
///
/// Broader than [`json_bridged_traversal_skip`] on purpose: it also refuses a count on a leaf the
/// IR never described, where emitting `.count` would be a guess.
pub(super) fn non_countable_leaf_count_skip(field_resolver: &FieldResolver, field: Option<&str>) -> Option<String> {
    let field = field?;
    let collection = COUNT_SUFFIXES
        .iter()
        .find_map(|suffix| field.strip_suffix(&format!(".{suffix}")))?;
    if collection.is_empty() || field_resolver.leaf_is_vec_via_swift_map(field_resolver.resolve(collection)) {
        return None;
    }
    Some(skip_line(field))
}

/// Whether the leaf's own getter returns `Option<..>`, so a caller chaining onto the rendered
/// accessor must write `?.` rather than `.`.
///
/// ~keep The accessor renderer deliberately omits the leaf `?` — it cannot know what will be
/// chained on — and a `?.` already in the chain only proves an ANCESTOR was optional. Reading the
/// ancestor's `?` as evidence that the leaf was unwrapped emitted `.toString()` against an
/// `Optional<RustString>` leaf, which has no such member. `false` when the IR did not describe the
/// leaf, which preserves the pre-existing behaviour for unmapped fields.
pub(super) fn leaf_getter_is_optional(field_resolver: &FieldResolver, field: Option<&str>) -> bool {
    field
        .filter(|f| !f.is_empty())
        .and_then(|f| field_resolver.swift_leaf_getter_is_optional(f))
        .unwrap_or(false)
}

fn skip_line(field: &str) -> String {
    format!(
        "        // skipped: {}\n",
        FieldSkip::CountOnJsonBridgedLeafInSwift.message(field)
    )
}
