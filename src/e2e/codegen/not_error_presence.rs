//! The one place that decides whether a `not_error` assertion may render an explicit
//! "the call succeeded" presence check (`assertNotNull`, `Assert.NotNull`,
//! `expect(x).toBeDefined()`, and equivalents).
//!
//! ~keep `not_error` is a filler assertion: an uncaught exception/panic already fails the test,
//! so `not_error`'s own presence check exists only to keep a fixture whose sole assertion is
//! `not_error` from being vacuous (`inert_example` exists specifically to catch that shape).
//! Once that reasoning is spelled out, two independent conditions make the presence check unsafe,
//! and either alone is reason enough to suppress it:
//!
//! - `result_is_option`: the call's own return type says a legitimate absent value (`None`) is
//!   possible on success (e.g. detecting a language from empty content returning `None`), so
//!   presence is not guaranteed regardless of what else the fixture asserts.
//! - a sibling assertion already exists beside `not_error`: it already gives the test real,
//!   non-vacuous coverage, so the filler is unneeded — and on an `Option<T>` result it would
//!   actively contradict a sibling `is_empty`/`not_empty` check on the same bare value (wasm-
//!   bindgen maps `None` -> `undefined`, which fails an unconditional `toBeDefined()`; NAPI's
//!   `None` -> `null` only dodged this by accident, since `null !== undefined`).
//!
//! This rule was discovered and independently re-fixed, under a different flag name each time,
//! in swift (`bare_result_is_option`), kotlin (`bare_result_is_option`, twice — once correctly,
//! once via a doc comment that falsely claimed the fix already existed), r
//! (`bare_result_is_option`), typescript (`has_other_assertions`), csharp (`has_other_assertions`),
//! elixir (`has_other_assertions`), and java (`result_is_option && bare_field`) — seven backends
//! independently discovering the same rule (alef #165 and its follow-ups) is the defect this
//! module exists to end: a backend calls [`may_assert_presence`] once and gets the correct
//! answer, instead of re-deriving it.
//!
//! Backends keep control of *how* to render the presence check (or its absence — a comment, a
//! no-op, a different fallback assertion); this module decides only *whether*.

use crate::e2e::fixture::Fixture;

/// Whether a `not_error` assertion belonging to `fixture` may render an explicit "the call
/// succeeded" presence check.
///
/// `result_is_option` is the call's already-resolved (backend override applied) fact that the
/// wrapped Rust function returns `Option<T>`. `not_error` itself never carries a field — every
/// backend's own `not_error` handling documents this — so no field argument is needed here: a
/// `not_error` assertion is always a bare check on the whole result.
pub fn may_assert_presence(fixture: &Fixture, result_is_option: bool) -> bool {
    if result_is_option {
        return false;
    }
    fixture.assertions.len() <= 1
}

#[cfg(test)]
mod tests {
    use super::may_assert_presence;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn fixture_with(assertion_types: &[&str]) -> Fixture {
        Fixture {
            assertions: assertion_types
                .iter()
                .map(|assertion_type| Assertion {
                    assertion_type: (*assertion_type).to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The baseline the whole rule protects: a fixture whose only assertion is `not_error`
    /// must still get a real, non-vacuous check when nothing about the call's shape makes that
    /// check unsafe.
    #[test]
    fn sole_not_error_on_a_non_option_result_may_assert_presence() {
        assert!(may_assert_presence(&fixture_with(&["not_error"]), false));
    }

    /// Regression (the C#/TypeScript/Elixir gap this module closes): `not_error` as the
    /// fixture's *sole* assertion on an `Option<T>`-returning call must not assert presence —
    /// `None` is a legitimate success outcome, and `has_other_assertions`-only guards let this
    /// case through unfixed because there is no sibling assertion to detect.
    #[test]
    fn sole_not_error_on_an_option_result_must_not_assert_presence() {
        assert!(!may_assert_presence(&fixture_with(&["not_error"]), true));
    }

    /// The originally-reported shape (alef #165): `not_error` paired with a sibling `is_empty`
    /// on a bare `Option<T>` result must not assert presence, or the pair can never both pass.
    #[test]
    fn not_error_beside_a_sibling_assertion_on_an_option_result_must_not_assert_presence() {
        assert!(!may_assert_presence(&fixture_with(&["not_error", "is_empty"]), true));
    }

    /// A sibling assertion already gives the test real coverage, so the `not_error` filler is
    /// unneeded even when the result is not `Option`-shaped — consistent behavior across every
    /// backend, not just the ones whose result happens to be optional.
    #[test]
    fn not_error_beside_a_sibling_assertion_on_a_non_option_result_must_not_assert_presence() {
        assert!(!may_assert_presence(&fixture_with(&["not_error", "is_true"]), false));
    }
}
