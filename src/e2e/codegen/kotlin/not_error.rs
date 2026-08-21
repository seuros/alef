//! Rendering for the `not_error` assertion type, split out of `assertions.rs` (already over
//! the file-size cap) so this fix does not grow it further.

use std::fmt::Write as FmtWrite;

/// Render the `not_error` assertion: a visible, real check that the call succeeded, rather
/// than the vacuous body that used to be emitted here.
///
/// ~keep A `not_error`-only fixture used to render nothing on the theory that the call having
/// succeeded without throwing already proves it: every sibling backend (java, csharp,
/// typescript, swift, python, elixir) carried the identical reasoning and the identical bug,
/// since fixed -- see `java/assertions.rs`'s `not_error` doc comment for the full history. An
/// uncaught exception does fail the test, but a fixture whose only assertion is `not_error`
/// must still leave a real, visible assertion instead of a vacuous body: `inert_example` exists
/// specifically to catch a generated example that is green because it asserts nothing, and this
/// shape was invisible to it before the fix landed for the other backends -- it surfaced as 9
/// distinct kotlin fixtures in a real downstream consumer's `alef e2e generate` run, each
/// logged individually once the inert-example ledger started naming refusals.
///
/// `assertNotNull` (not `assertTrue(x != null, ...)`) because Kotlin's compiler flags an
/// explicit `!= null` comparison against a statically non-nullable type as "condition is always
/// true"; `assertNotNull`'s `T?` parameter accepts a non-null `T` without tripping that check.
/// For streaming fixtures, assert on the drained `chunks` list (bound by `collect_snippet`
/// before this runs) instead of `result_var`, matching every other streaming assertion in
/// `assertions.rs`.
///
/// `bare_result_is_option`: a bare `T?` result (no field path) may legitimately be `null` on
/// success -- e.g. detecting a language from empty content returning `null` is not an error.
/// Asserting `assertNotNull` there directly contradicts a paired `is_empty`/`not_empty`
/// assertion on the same bare result, which correctly emits `assertNull`/`assertNotNull`
/// itself (see `assertions.rs`'s own `bare_result_is_option` branches). This module's own doc
/// comment previously claimed Kotlin already treated `not_error` as inert in this shape --
/// it did not; `render_not_error` took no such parameter. Mirrors the fix already shipped for
/// swift (`swift/not_error_assertion.rs`). ~keep
pub(super) fn render_not_error(out: &mut String, result_var: &str, bare_result_is_option: bool, is_streaming: bool) {
    if bare_result_is_option {
        let _ = writeln!(
            out,
            "        // not_error: covered by the bare Optional's own assertion"
        );
    } else if is_streaming {
        let _ = writeln!(
            out,
            "        assertTrue(chunks.isNotEmpty(), \"expected at least one streamed chunk\")"
        );
    } else {
        let _ = writeln!(out, "        assertNotNull({result_var}, \"expected non-null result\")");
    }
}

#[cfg(test)]
mod tests {
    use super::render_not_error;

    /// The regression this whole module exists for: `liter-llm`'s `search_basic` fixture (and
    /// eight siblings) declared only `{"type": "not_error"}` and rendered a Kotlin test body
    /// with no executable line at all -- green because it asserted nothing, not because the
    /// call worked. ~keep
    #[test]
    fn non_streaming_renders_a_real_assertion_on_the_result_variable() {
        let mut out = String::new();
        render_not_error(&mut out, "result", false, false);
        assert_eq!(out, "        assertNotNull(result, \"expected non-null result\")\n");
    }

    /// `result_var` for a streaming fixture is not a value `assertNotNull` can usefully check
    /// (it may be the stream/iterator itself, not the drained list), so this must route through
    /// `chunks` instead, like every other streaming assertion in `assertions.rs`.
    #[test]
    fn streaming_asserts_on_the_drained_chunks_list_not_the_result_variable() {
        let mut out = String::new();
        render_not_error(&mut out, "result", false, true);
        assert_eq!(
            out,
            "        assertTrue(chunks.isNotEmpty(), \"expected at least one streamed chunk\")\n"
        );
        assert!(
            !out.contains("result"),
            "streaming must not reference result_var: got {out}"
        );
    }

    /// Regression: a bare `T?` result (no field path) must not get an `assertNotNull` from
    /// `not_error` -- `null` is a valid non-error outcome, and a paired `is_empty`/`not_empty`
    /// assertion on the same bare result already emits its own `assertNull`/`assertNotNull`.
    /// Before this fix, `render_not_error` had no `bare_result_is_option` parameter and always
    /// emitted `assertNotNull(result, ...)`, producing a contradictory pair with a sibling
    /// `is_empty` that can never both pass. ~keep
    #[test]
    fn bare_optional_result_emits_no_not_null_assertion() {
        let mut out = String::new();
        render_not_error(&mut out, "result", true, false);
        assert!(
            !out.contains("assertNotNull(result"),
            "bare Optional result must not assert non-null from not_error: got {out}"
        );
    }
}
