//! `not_error` assertion rendering, split out of `swift/assertions.rs` (already over the
//! repo's 1,000-line cap — see `file-modularization` in CLAUDE.md) so this fix's regression
//! coverage has somewhere to live without growing either oversized file. ~keep

use std::fmt::Write as FmtWrite;

/// Render the `not_error` assertion.
///
/// An uncaught exception already fails the test, but `test_method.rs` only ever used
/// `has_not_error_assertion` to decide whether to BIND the result (`let result = ...` vs
/// `_ = ...`), never to assert on it — a fixture whose only assertion was `not_error` bound
/// `result` and then never referenced it in an `XCTAssert*` call, leaving the test vacuous.
/// Emit a real assertion instead, for the cases where one is actually meaningful:
///
/// - `returns_void` calls bind no `result` at all (see `test_method.rs`'s `if
///   call_config.returns_void` branch), so there's nothing to assert on — the exception path
///   already covers it.
/// - Streaming fixtures assert on the drained `chunks` array (bound by the collect snippet
///   before this runs) rather than the raw stream.
/// - A bare `Optional<T>` result (`bare_result_is_option`) may legitimately be `nil` on
///   success — `detectLanguageFromContent("")` returning `nil` is not an error. Asserting
///   `XCTAssertNotNil` there is simply wrong, and directly contradicts a paired `is_empty`/
///   `is_true` assertion on the same bare result, which correctly emits `XCTAssertNil`.
///   Zig (`zig/assertions.rs`) and Kotlin (`kotlin/assertions.rs`) already treat `not_error`
///   as inert in this shape; Swift did not, and emitted both `XCTAssertNotNil(result)` and
///   `XCTAssertNil(result)` back to back — an assertion pair that can never pass. ~keep
pub(super) fn render_not_error_assertion(
    out: &mut String,
    result_var: &str,
    bare_result_is_option: bool,
    is_streaming: bool,
    returns_void: bool,
) {
    if bare_result_is_option {
        let _ = writeln!(out, "        // not_error: covered by try propagation");
    } else if returns_void {
        // No variable to assert on; the exception path already covers this.
    } else if is_streaming {
        let _ = writeln!(out, "        XCTAssertNotNil(chunks)");
    } else {
        let _ = writeln!(out, "        XCTAssertNotNil({result_var})");
    }
}

#[cfg(test)]
mod tests {
    use super::render_not_error_assertion;

    /// Regression: a bare `Optional<T>` result (no field path) must not get an
    /// `XCTAssertNotNil` from `not_error` — `nil` is a valid non-error outcome, and a
    /// paired `is_empty`/`is_true` assertion on the same bare result already emits
    /// `XCTAssertNil`. Before this fix, `render_not_error_assertion` had no
    /// `bare_result_is_option` branch and always emitted `XCTAssertNotNil(result)`,
    /// producing a contradictory `XCTAssertNotNil(result)` + `XCTAssertNil(result)` pair
    /// that can never pass (seen live in tslp's `testErrorDetectContentEmpty`).
    #[test]
    fn bare_optional_result_emits_no_not_nil_assertion() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, "result", true, false, false);

        assert!(
            !out.contains("XCTAssertNotNil(result)"),
            "bare Optional result must not assert not-nil from `not_error`: {out}"
        );
    }

    /// Non-optional bare results still get a real assertion, so a fixture whose only
    /// assertion is `not_error` stays non-vacuous.
    #[test]
    fn non_optional_result_still_asserts_not_nil() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, "result", false, false, false);

        assert_eq!(out, "        XCTAssertNotNil(result)\n");
    }

    #[test]
    fn streaming_result_asserts_on_chunks() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, "result", false, true, false);

        assert_eq!(out, "        XCTAssertNotNil(chunks)\n");
    }

    #[test]
    fn void_result_emits_nothing() {
        let mut out = String::new();

        render_not_error_assertion(&mut out, "result", false, false, true);

        assert!(out.is_empty());
    }
}
