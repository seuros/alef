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
/// WHETHER `not_error` may assert presence at all -- as opposed to staying inert because a
/// bare `T?` result (`result_is_option`, no field path) may legitimately be `null` on success,
/// or because a sibling assertion already gives the test real coverage -- is decided once,
/// centrally, by `not_error_presence::may_assert_presence` (shared with typescript, csharp,
/// elixir, java; see that module's doc for why this was reinvented independently seven times,
/// including once via a doc comment on this very function that falsely claimed Kotlin already
/// handled the bare-Optional case when `render_not_error` in fact took no such parameter).
/// `render_not_error` only decides how to render the check, not whether. ~keep
pub(super) fn render_not_error(out: &mut String, result_var: &str, may_assert_presence: bool, is_streaming: bool) {
    if is_streaming {
        let _ = writeln!(
            out,
            "        assertTrue(chunks.isNotEmpty(), \"expected at least one streamed chunk\")"
        );
    } else if may_assert_presence {
        let _ = writeln!(out, "        assertNotNull({result_var}, \"expected non-null result\")");
    } else {
        let _ = writeln!(
            out,
            "        // not_error: covered by the bare Optional's own assertion"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::render_not_error;
    use crate::e2e::codegen::not_error_presence::may_assert_presence;
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

    /// The regression this whole module exists for: `liter-llm`'s `search_basic` fixture (and
    /// eight siblings) declared only `{"type": "not_error"}` and rendered a Kotlin test body
    /// with no executable line at all -- green because it asserted nothing, not because the
    /// call worked. ~keep
    #[test]
    fn non_streaming_renders_a_real_assertion_on_the_result_variable() {
        let mut out = String::new();
        render_not_error(&mut out, "result", true, false);
        assert_eq!(out, "        assertNotNull(result, \"expected non-null result\")\n");
    }

    /// `result_var` for a streaming fixture is not a value `assertNotNull` can usefully check
    /// (it may be the stream/iterator itself, not the drained list), so this must route through
    /// `chunks` instead, like every other streaming assertion in `assertions.rs`.
    #[test]
    fn streaming_asserts_on_the_drained_chunks_list_not_the_result_variable() {
        let mut out = String::new();
        render_not_error(&mut out, "result", true, true);
        assert_eq!(
            out,
            "        assertTrue(chunks.isNotEmpty(), \"expected at least one streamed chunk\")\n"
        );
        assert!(
            !out.contains("result"),
            "streaming must not reference result_var: got {out}"
        );
    }

    /// Regression: when the caller's `may_assert_presence` is `false` (as
    /// `not_error_presence::may_assert_presence` computes for a bare `T?` result, among other
    /// cases), `not_error` must not get an `assertNotNull` -- `null` is a valid non-error
    /// outcome, and a paired `is_empty`/`not_empty` assertion on the same bare result already
    /// emits its own `assertNull`/`assertNotNull`. Before this fix, `render_not_error` had no
    /// such parameter and always emitted `assertNotNull(result, ...)`, producing a contradictory
    /// pair with a sibling `is_empty` that can never both pass. ~keep
    #[test]
    fn presence_not_permitted_emits_no_not_null_assertion() {
        let mut out = String::new();
        render_not_error(&mut out, "result", false, false);
        assert!(
            !out.contains("assertNotNull(result"),
            "may_assert_presence: false must not assert non-null from not_error: got {out}"
        );
    }

    /// End-to-end through the real shared decision, not a hand-picked literal: a fixture whose
    /// only assertion is `not_error` on a bare `Option<T>`-returning call
    /// (`not_error_presence::may_assert_presence(&fixture, true)`) must stay inert, exactly like
    /// the swift/java fix -- this is the shape `bare_result_is_option` protected before the
    /// unification and must keep protecting after it.
    #[test]
    fn sole_not_error_on_an_option_result_via_the_shared_decision_stays_inert() {
        let fixture = fixture_with(&["not_error"]);
        let may_assert = may_assert_presence(&fixture, true);
        let mut out = String::new();
        render_not_error(&mut out, "result", may_assert, false);
        assert!(
            !out.contains("assertNotNull(result"),
            "bare Option<T> result must not assert non-null from not_error even as the sole \
             assertion: got {out}"
        );
    }
}
