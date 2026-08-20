//! Regression coverage for the Zig opaque-handle accessor defect: `render_zig_with_optionals`
//! could only ever emit `.field`, never `.field()`, so a fixture asserting on a field that the
//! generated Zig binding exposes as a real method call on an opaque handle (an FFI getter, e.g.
//! `tree.language()`) produced `result.language` — a path the Zig struct does not declare,
//! since only the getter METHOD exists.
//!
//! These tests drive `FieldResolver::accessor("zig", ...)` directly, mirroring the Rust
//! accessor's own `method_calls`/`result_fields` regression coverage in `tests.rs`
//! (`render_rust_with_result_fields_overrides_method_calls`): a path in `method_calls` and NOT
//! in `result_fields` is a real method call and gets `()`; a path in both keeps the pre-existing
//! tagged-union-variant shape (plain `.field`, no `()`, no forced `.?`).

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn resolver(optional: &[&str], method_calls: &[&str], result_fields: &[&str]) -> FieldResolver {
    let optional: HashSet<String> = optional.iter().map(|s| s.to_string()).collect();
    let method_calls: HashSet<String> = method_calls.iter().map(|s| s.to_string()).collect();
    let result_fields: HashSet<String> = result_fields.iter().map(|s| s.to_string()).collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &result_fields,
        &HashSet::new(),
        &method_calls,
    )
}

/// The defect itself: a field in `method_calls` and not in `result_fields` — an opaque-handle
/// getter — must render as a real Zig method call. Before the fix, `render_zig_with_optionals`
/// had no code path that could ever append `()`, so this produced `result.language` against a
/// Zig type whose only accessor is `pub fn language(self) ...`.
#[test]
fn opaque_handle_getter_field_renders_as_a_real_zig_method_call() {
    let r = resolver(&[], &["language"], &[]);
    assert_eq!(r.accessor("language", "zig", "result"), "result.language()");
}

/// Positive control mirroring the Rust accessor's own override: a path in BOTH `method_calls`
/// and `result_fields` keeps the pre-existing tagged-union-variant shape — plain dot access,
/// no `()`. This is the exact `DocumentResult.content: String` scenario `render_rust_with_
/// optionals`'s doc names, carried over to Zig so the fix does not turn every `method_calls`
/// entry into a method call unconditionally.
#[test]
fn field_also_classified_as_result_field_keeps_plain_dot_access() {
    let r = resolver(&[], &["content"], &["content"]);
    assert_eq!(r.accessor("content", "zig", "result"), "result.content");
}

/// An optional opaque-handle getter unwraps with `.?` AFTER the call, since the getter itself
/// returns `?T` in this scenario.
#[test]
fn optional_opaque_handle_getter_unwraps_after_the_call() {
    let r = resolver(&["language"], &["language"], &[]);
    assert_eq!(r.accessor("language", "zig", "result"), "result.language().?");
}

/// Negative control: a field with no `method_calls` entry at all is unaffected — plain struct
/// field access, exactly as before this fix.
#[test]
fn plain_struct_field_is_unaffected() {
    let r = resolver(&[], &[], &[]);
    assert_eq!(r.accessor("active", "zig", "result"), "result.active");
}

/// The pre-existing tagged-union-variant behaviour this fix must not disturb: a path in both
/// `method_calls` and `result_fields` that is ALSO in `optional_fields` still suppresses `.?`
/// (Zig tagged-union variant access does not need it), matching the doc comment's example.
#[test]
fn tagged_union_variant_style_field_still_suppresses_forced_unwrap() {
    let r = resolver(&["format.excel"], &["format.excel"], &["format.excel"]);
    assert_eq!(r.accessor("format.excel", "zig", "result"), "result.format.excel");
}
