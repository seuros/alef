//! Regression coverage for the `not_error` presence-assertion defect (alef #165, C# arm).
//!
//! `render_assertion`'s `not_error` arm unconditionally emitted `Assert.NotNull(result)`, even
//! when a sibling `is_empty` assertion on the same bare result declared the call's success path
//! can legitimately return nothing (`Option<T> -> None -> C# null`). A fixture pairing the two
//! produced a contradictory `Assert.NotNull(result)` + `Assert.True(string.IsNullOrEmpty(...))`
//! (or `Assert.Empty(result)`) pair that can never pass. Mirrors the TypeScript fix
//! (`typescript/assertions.rs`'s `has_other_assertions` guard) applied here via the same
//! parameter name and reasoning.
//!
//! Lives in its own file rather than growing `csharp/assertions.rs`: that file is already over
//! the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use super::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    )
}

fn not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }
}

fn is_empty_assertion() -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn render(assertion: &Assertion, has_other_assertions: bool) -> String {
    let resolver = empty_resolver();
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        "SampleClass",
        "SampleException",
        &resolver,
        false,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        has_other_assertions,
    );
    out
}

/// The regression this file exists for: a fixture whose assertions are `not_error` +
/// `is_empty` on a bare `Option<T>` result must not assert presence from `not_error` — that
/// would contradict `is_empty`'s `Assert.Empty`/`Assert.True(IsNullOrEmpty(...))` check on the
/// same variable and the pair could never pass.
#[test]
fn not_error_paired_with_is_empty_does_not_assert_presence() {
    let out = render(&not_error_assertion(), true);
    assert_eq!(
        out, "",
        "not_error must render nothing when a sibling assertion exists; got: {out}"
    );
}

/// Control: a fixture whose only assertion is `not_error` must still emit a real,
/// non-vacuous check — the guard must not silence the fallback unconditionally.
#[test]
fn not_error_as_sole_assertion_still_asserts_presence() {
    let out = render(&not_error_assertion(), false);
    assert_eq!(out, "        Assert.NotNull(result);\n");
}

/// End-to-end: rendering `not_error` then `is_empty` (the actual fixture order) produces only
/// the `is_empty` check, not both.
#[test]
fn not_error_then_is_empty_in_sequence_emits_only_is_empty() {
    let mut out = String::new();
    let resolver = empty_resolver();
    for assertion in [&not_error_assertion(), &is_empty_assertion()] {
        render_assertion(
            &mut out,
            assertion,
            "result",
            "SampleClass",
            "SampleException",
            &resolver,
            false,
            false,
            false,
            false,
            &std::collections::HashMap::new(),
            true,
        );
    }
    assert!(
        !out.contains("Assert.NotNull(result)"),
        "not_error must not assert not-null alongside is_empty; got: {out}"
    );
    assert!(!out.is_empty(), "is_empty must still render its own check; got: {out}");
}
