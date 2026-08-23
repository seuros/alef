//! Crate-level Rust attribute formatting for generated binding crates.
//!
//! Kept apart from the IR-shaped helpers in [`super::shared`]: these functions manipulate
//! attribute *text*, not IR, and re-export through `shared` so existing call paths hold.

use std::collections::HashSet;

/// Format extra clippy allows for insertion into generated Rust binding files.
///
/// Accepts bare lint names (`"single_match"`) or `clippy::`-prefixed names
/// (`"clippy::single_match"`); both forms are normalised to the `clippy::` prefix.
///
/// Returns `None` when `extras` is empty — callers must skip emission entirely in
/// that case so output is byte-identical to the no-config baseline.
///
/// Returns `Some(attr)` where `attr` is the inner content of an `allow(...)` call,
/// e.g. `"allow(clippy::single_match, clippy::collapsible_match)"`.  Pass this
/// directly to [`crate::codegen::builder::RustFileBuilder::add_inner_attribute`]
/// or format it into a raw `#![allow(...)]` attribute string.
///
/// `already_emitted` is the attribute text emitted above this call (each backend's
/// default allow block). Lints already present there are filtered out so the extra
/// block never re-allows a lint — a duplicate would trip clippy's
/// `duplicated_attributes` lint under `-D warnings`. Returns `None` when nothing new
/// remains, so callers skip emission entirely.
pub fn format_extra_clippy_allows(extras: &[String], already_emitted: &str) -> Option<String> {
    if extras.is_empty() {
        return None;
    }
    let already = collect_clippy_lints(already_emitted);
    let mut seen = HashSet::new();
    let normalized: Vec<String> = extras
        .iter()
        .map(|s| {
            let trimmed = s.trim();
            if trimmed.starts_with("clippy::") {
                trimmed.to_string()
            } else {
                format!("clippy::{trimmed}")
            }
        })
        .filter(|s| !already.contains(s.as_str()))
        .filter(|s| seen.insert(s.clone()))
        .collect();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("allow({})", normalized.join(", ")))
}

/// Format per-crate `crate_attributes` config entries for splicing into a generated
/// crate's `lib.rs`, one inner attribute per entry, in configured order.
///
/// Entries are raw attribute *bodies* (e.g. `recursion_limit = "256"`), not full
/// `#![...]` syntax — the same convention as [`format_extra_clippy_allows`]. Unlike
/// that function, entries are **not** merged into a single attribute: each one is a
/// distinct, unrelated attribute (`recursion_limit`, `feature(...)`, `warn(...)`, ...),
/// so merging them would be meaningless.
///
/// Well-formedness (non-empty, single-line, valid leading attribute path, not already
/// wrapped in `#![...]`) is validated once at config-resolve time — see
/// `crate::core::config::new_config::NewAlefConfig::resolve`. This function assumes
/// already-validated input and only trims incidental whitespace, matching the
/// historical behavior of `format_extra_clippy_allows`.
///
/// Returns an empty `Vec` when `attributes` is empty — callers must skip emission
/// entirely in that case so output is byte-identical to the no-config baseline.
pub fn format_crate_attributes(attributes: &[String]) -> Vec<String> {
    attributes
        .iter()
        .map(|attribute| attribute.trim().to_string())
        .collect()
}

/// Collect the `clippy::<lint>` tokens present in already-emitted attribute text,
/// used to de-duplicate extra clippy allows against a backend's default allow block.
fn collect_clippy_lints(text: &str) -> HashSet<&str> {
    const PREFIX: &str = "clippy::";
    let mut lints = HashSet::new();
    let mut rest = text;
    while let Some(idx) = rest.find(PREFIX) {
        let after = &rest[idx + PREFIX.len()..];
        let name_len = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let token_end = idx + PREFIX.len() + name_len;
        if name_len > 0 {
            lints.insert(&rest[idx..token_end]);
        }
        rest = &rest[token_end..];
    }
    lints
}
