//! `not_error` assertion rendering, split out of `r/assertions.rs` (already over the repo's
//! 1,000-line cap -- see `file-modularization` in CLAUDE.md) so this fix's regression coverage
//! has somewhere to live without growing an oversized file further. Mirrors
//! `swift/not_error_assertion.rs`'s split for the identical reason. ~keep
//!
//! ~keep The non-void arm used to emit a bare `expect_true(TRUE)` -- a testthat expectation
//! that cannot fail, i.e. no real check at all. The obvious replacement,
//! `expect_true(!is.null(result))`, is unsafe for two result shapes: a `result_is_simple`
//! extendr scalar return and a bare `result_is_option` (`Option<T>`) return can both
//! legitimately be R's "nothing here" on a *successful* call -- `Result<Option<T>, E>::Ok(None)`
//! is not an error, and asserting non-null there would fail correct binding behaviour. Swift's
//! `bare_result_is_option` (`swift/not_error_assertion.rs`) documents the identical trap for the
//! same reason.
//!
//! For those two shapes the real, failable check moves to the call site instead: `r/test_case.rs`
//! wraps the fallible call itself in testthat's `expect_no_error(...)` (verified empirically to
//! both propagate the call's return value on success, via `x <- expect_no_error(expr)`, and to
//! fail the test when `expr` raises) rather than asserting on a value whose emptiness is
//! sometimes the correct outcome. This module's `render` therefore renders nothing for either
//! shape -- not a vacuous placeholder, but a deliberate no-op because the real assertion already
//! ran one statement earlier. Every other (non-simple, non-option) shape gets the real,
//! non-vacuous `expect_true(!is.null(result))` check.

use std::fmt::Write as FmtWrite;

/// True when `result` may legitimately be R `NULL`/`NA` after a *successful* call under this
/// result shape, so a bare non-null check on it would reject correct behaviour.
///
/// Shared between this module's `render` (which must render nothing here rather than a false
/// check) and `r/test_case.rs` (which must wrap the call itself in `expect_no_error(...)` to
/// still leave a real check, and must not let its own vacuous-assertion fallback re-inject the
/// unsafe check this function exists to avoid). ~keep
pub(super) fn unsafe_for_null_check(result_is_simple: bool, result_is_option: bool) -> bool {
    result_is_simple || result_is_option
}

/// Render the `not_error` assertion for a non-void R call.
///
/// `returns_void` and `unsafe_for_null_check` are mutually exclusive real-check locations, not
/// two guards on the same one: a `returns_void` call binds no `result` at all (`test_case.rs`'s
/// `expect_no_error(function(...))`, no assignment), while an `unsafe_for_null_check` call still
/// binds `result` for other assertions to use but defers its `not_error` check to the same
/// `expect_no_error(...)` wrapper around the assignment's right-hand side. Both cases render
/// nothing here because the real, failable expectation already ran at the call site. ~keep
pub(super) fn render(
    out: &mut String,
    result_var: &str,
    returns_void: bool,
    result_is_simple: bool,
    result_is_option: bool,
) {
    if returns_void || unsafe_for_null_check(result_is_simple, result_is_option) {
        return;
    }
    let _ = writeln!(out, "  expect_true(!is.null({result_var}))");
}

#[cfg(test)]
mod tests {
    use super::{render, unsafe_for_null_check};

    /// The regression this module exists for: a fixture whose only assertion is `not_error`
    /// on an ordinary (non-simple, non-option) result must get a REAL testthat expectation,
    /// not `expect_true(TRUE)` -- an expectation that can never fail. ~keep
    #[test]
    fn ordinary_result_gets_a_real_non_null_assertion() {
        let mut out = String::new();
        render(&mut out, "result", false, false, false);
        assert_eq!(out, "  expect_true(!is.null(result))\n");
    }

    /// A `result_is_simple` extendr scalar return may legitimately be R's empty
    /// representation on success (e.g. an `Option<String>::None` surfaces as
    /// `NA_character_`), so `not_error` must not assert non-null here -- the real check
    /// already ran at the call site via `expect_no_error(...)`.
    #[test]
    fn simple_result_renders_nothing_here_not_a_false_null_check() {
        let mut out = String::new();
        render(&mut out, "result", false, true, false);
        assert!(
            out.is_empty(),
            "the call-site expect_no_error is the real check, got: {out}"
        );
    }

    /// A bare `Option<T>` result may be R `NULL` on a successful `Ok(None)`; asserting
    /// non-null would reject correct behaviour, exactly the trap `swift/not_error_assertion.rs`
    /// documents for `bare_result_is_option`.
    #[test]
    fn option_result_renders_nothing_here_not_a_false_null_check() {
        let mut out = String::new();
        render(&mut out, "result", false, false, true);
        assert!(
            out.is_empty(),
            "the call-site expect_no_error is the real check, got: {out}"
        );
    }

    #[test]
    fn void_result_renders_nothing_here_either() {
        let mut out = String::new();
        render(&mut out, "result", true, false, false);
        assert!(out.is_empty());
    }

    #[test]
    fn unsafe_for_null_check_is_true_for_either_shape() {
        assert!(unsafe_for_null_check(true, false));
        assert!(unsafe_for_null_check(false, true));
        assert!(!unsafe_for_null_check(false, false));
    }
}
