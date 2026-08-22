//! Regression coverage for `not_error` contradicting a bare `@Nullable T` result's own
//! `is_empty`/`not_empty` assertion.
//!
//! ~keep A fixture can legitimately pair `not_error` with `is_empty` on a bare Option<T>-
//! returning call whose success path returns nothing (e.g. detecting a language from empty
//! content -> `None`). The Java facade maps that to `@Nullable T` (`.orElse(null)`), and
//! `assertions.rs`'s `result_is_option && bare_field` block already special-cases `is_empty`
//! (`assertNull`) and `not_empty` (`assertNotNull`) for this shape; `not_error` itself is
//! handled by the general `not_error` arm further down, gated by a `not_error_may_assert_presence`
//! boolean computed once, centrally, by `not_error_presence::may_assert_presence` (shared with
//! typescript, csharp, elixir, kotlin — see that module's doc for why this was reinvented
//! independently seven times). These tests drive `render_assertion` — the real generator, not a
//! hand-written mirror of it — with flags produced by the real shared function.
//!
//! Lives in its own file rather than growing `assertions.rs`: that file is already over the
//! repo's 1,000-line cap (see `file-modularization` in CLAUDE.md).

use super::assertions::render_assertion;
use crate::e2e::codegen::not_error_presence::may_assert_presence;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
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

fn fixture_with(assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        assertions,
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn render(assertion: &Assertion, result_is_option: bool, not_error_may_assert_presence: bool) -> String {
    let resolver = empty_resolver();
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        "Result",
        &resolver,
        false,
        false,
        result_is_option,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        not_error_may_assert_presence,
    );
    out
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

/// The regression this file exists for: `not_error` on a bare `Option<T>` result must not
/// emit `assertNotNull` — it must stay inert, exactly like the swift fix.
#[test]
fn not_error_on_bare_option_result_emits_no_assert_not_null() {
    let fixture = fixture_with(vec![not_error_assertion()]);
    let may_assert = may_assert_presence(&fixture, true);
    let out = render(&not_error_assertion(), true, may_assert);
    assert!(
        !out.contains("assertNotNull(result"),
        "bare Option<T> result must not assert non-null from not_error: got {out}"
    );
}

/// Combining both assertions, in fixture-declaration order, must never render a contradictory
/// `assertNull` + `assertNotNull` pair on the same variable.
#[test]
fn not_error_paired_with_is_empty_does_not_contradict_it() {
    let fixture = fixture_with(vec![not_error_assertion(), is_empty_assertion()]);
    let may_assert = may_assert_presence(&fixture, true);
    let mut out = String::new();
    for assertion in [not_error_assertion(), is_empty_assertion()] {
        out.push_str(&render(&assertion, true, may_assert));
    }
    assert!(
        !out.contains("assertNotNull(result"),
        "not_error must not contradict a sibling is_empty on a bare Option<T> result: got {out}"
    );
    assert!(
        out.contains("assertNull(result, \"expected empty value\");"),
        "is_empty must still render its own null check: got {out}"
    );
}

/// Control: a non-Option bare result must keep the pre-existing, non-vacuous behavior —
/// `not_error` alone still needs a real assertion when the result can never legitimately be
/// null on success.
#[test]
fn not_error_on_non_option_result_still_asserts_not_null() {
    let fixture = fixture_with(vec![not_error_assertion()]);
    let may_assert = may_assert_presence(&fixture, false);
    let out = render(&not_error_assertion(), false, may_assert);
    assert!(
        out.contains("assertNotNull(result, \"expected non-null response\");"),
        "got: {out}"
    );
}
