//! Regression tests for a one-sided-trim bug in the zig `equals` assertion.
//!
//! `std.mem.trim` wrapped the ACTUAL value while the fixture `expected` literal was emitted
//! verbatim. Fixture expectations may legitimately end in `\n`, so trimming only one side made
//! those assertions impossible to satisfy — and trimming both would silently mask real
//! trailing-whitespace regressions. Equals is exact: neither side is normalized. This matches
//! the contract every other e2e backend already enforces (see the sibling
//! `render_assertion_equals_string_compares_exactly_without_trim` tests in the php, ruby,
//! typescript and elixir assertion modules).
//!
//! Split into its own file rather than added to `zig/assertions.rs`: that file is already over
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test coverage
//! goes into a fresh module instead of growing it. ~keep

use super::assertions::render_json_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn equals_assertion(field: Option<&str>, value: &str) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: field.map(str::to_string),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Assertion::default()
    }
}

fn render(assertion: &Assertion) -> String {
    let mut out = String::new();
    render_json_assertion(&mut out, assertion, "result", &empty_resolver(), false);
    out
}

/// The emitted assertion must compare the raw `.string` against the verbatim literal. A trim on
/// either side is the bug: with the expected literal keeping its `\n` and the actual value
/// stripped of it, the assertion could never pass for any value at all.
#[test]
fn json_equals_string_compares_exactly_without_trim() {
    let out = render(&equals_assertion(Some("content"), "Hello World\n"));
    assert_eq!(
        out, "    try testing.expectEqualStrings(\"Hello World\\n\", result.object.get(\"content\").?.string);\n",
        "emitted assertion drifted: {out}"
    );
}

/// Control for the trim fix: the tightened contract must still DISCRIMINATE values that differ
/// only in trailing whitespace. If either side were normalized, the emitted assertion for
/// "hello\n" and for "hello" would be identical and a real trailing-newline regression would
/// pass unnoticed.
#[test]
fn json_equals_still_discriminates_trailing_whitespace() {
    let with_newline = render(&equals_assertion(Some("content"), "hello\n"));
    let without_newline = render(&equals_assertion(Some("content"), "hello"));
    assert_ne!(
        with_newline, without_newline,
        "trailing newline must still change the emitted assertion"
    );
}

/// The `metadata.format` discriminated-union path builds its own comparison against a
/// `_fmt_display` local instead of going through the template, so it carried a second copy of
/// the same one-sided trim.
#[test]
fn discriminated_format_equals_compares_exactly_without_trim() {
    let out = render(&equals_assertion(Some("metadata.format"), "PNG\n"));
    assert!(
        out.contains("try testing.expectEqualStrings(\"PNG\\n\", _fmt_display);"),
        "equals must not trim either side; got: {out}"
    );
    assert!(
        !out.contains("std.mem.trim"),
        "equals must not trim either side; got: {out}"
    );
}
