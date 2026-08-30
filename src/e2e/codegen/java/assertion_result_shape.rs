//! Assertion overrides driven by the overall *shape* of a Java call's result value --
//! `Option`-wrapped, `byte[]`, or a `not_error` assertion -- rather than by any specific
//! field. Each function reports whether it emitted an assertion (`true`) so the caller in
//! `assertions.rs` knows to stop, or falls through (`false`) so the caller continues with
//! its normal field-based handling.
//!
//! Split out of `assertions.rs` (file-modularization cap): see that file's module doc.

use crate::e2e::fixture::Assertion;

pub(super) fn try_bare_option_result_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    result_is_option: bool,
) -> bool {
    // Bare-result is_empty / not_empty on Option<T> returns: the Java facade exposes
    // these as `@Nullable T` (via `.orElse(null)`) rather than `Optional<T>`, so the
    // template's `.isEmpty()` call would not compile for record types. Emit a
    // null-check instead — mirrors the kotlin / zig codegen behaviour.
    //
    // `not_error` is deliberately absent from this match: WHETHER it may assert presence is
    // decided once, centrally, by the caller via `not_error_presence::may_assert_presence`
    // (which already accounts for `result_is_option`) and handled in the general `not_error`
    // arm below, alongside every other backend's identical decision point. ~keep
    let bare_field = assertion.field.as_deref().is_none_or(str::is_empty);
    if result_is_option && bare_field {
        match assertion.assertion_type.as_str() {
            "is_empty" => {
                out.push_str(&format!(
                    "        assertNull({result_var}, \"expected empty value\");\n"
                ));
                return true;
            }
            "not_empty" => {
                out.push_str(&format!(
                    "        assertNotNull({result_var}, \"expected non-empty value\");\n"
                ));
                return true;
            }
            _ => {}
        }
    }
    false
}

pub(super) fn try_bytes_result_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    result_is_bytes: bool,
) -> bool {
    // Byte-buffer returns: emit length-based assertions instead of struct-field
    // accessors. The result is `byte[]`, which has no `isEmpty()`/struct-field methods.
    // Field paths on byte-buffer results (e.g. `audio`, `content`) are pseudo-fields
    // referencing the buffer itself — treat them the same as no-field assertions.
    if result_is_bytes {
        match assertion.assertion_type.as_str() {
            "not_empty" => {
                out.push_str(&format!(
                    "        assertTrue({result_var}.length > 0, \"expected non-empty value\");\n"
                ));
                return true;
            }
            "is_empty" => {
                out.push_str(&format!(
                    "        assertEquals(0, {result_var}.length, \"expected empty value\");\n"
                ));
                return true;
            }
            "count_equals" | "length_equals" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    out.push_str(&format!("        assertEquals({n}, {result_var}.length);\n"));
                }
                return true;
            }
            "count_min" | "length_min" => {
                if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                    out.push_str(&format!(
                        "        assertTrue({result_var}.length >= {n}, \"expected length >= {n}\");\n"
                    ));
                }
                return true;
            }
            "not_error" => {
                // Use the statically-imported assertion (org.junit.jupiter.api.Assertions.*)
                // so we don't need a separate FQN import of the `Assertions` class.
                out.push_str(&format!(
                    "        assertNotNull({result_var}, \"expected non-null byte[] response\");\n"
                ));
                return true;
            }
            _ => {
                out.push_str(&format!(
                    "        // skipped: assertion type '{}' not supported on byte[] result\n",
                    assertion.assertion_type
                ));
                return true;
            }
        }
    }
    false
}

pub(super) fn try_not_error_result_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    returns_void: bool,
    is_streaming: bool,
    not_error_may_assert_presence: bool,
) -> bool {
    // `not_error` never carries a `field` and has no `java/assertion.jinja` branch —
    // that template's if/elif chain has no `else`, so before this the call silently
    // rendered nothing. An uncaught exception already fails the `@Test` method, but a
    // fixture whose only assertion is `not_error` must still leave a real, visible
    // assertion instead of a vacuous body. Mirrors the `assertNotNull` idiom the
    // byte[] branch above already uses. For streaming fixtures, assert on the
    // drained `chunks` list (bound by `collect_snippet` before this runs) rather
    // than the raw `result_var`, so a lazily-consumed stream that errors only on
    // iteration is still caught. `returns_void` calls bind no `result_var` at all
    // (`java/test_method.jinja`'s `{% if returns_void %}` branch calls without
    // assigning), so asserting on a variable here would not compile — that case is
    // handled at the call-emission site instead: `test_method.rs`'s `void_not_error`
    // flag wraps `call_expr` itself in `assertDoesNotThrow(() -> ...)`, so this arm
    // stays a no-op purely because the real assertion lives one level up, not because
    // nothing is asserted. WHETHER the plain (non-void, non-streaming) case below may
    // assert presence at all is decided once, centrally, by
    // `not_error_presence::may_assert_presence` — this arm only decides how. ~keep
    if assertion.assertion_type == "not_error" {
        if returns_void {
            // Handled by `test_method.rs`'s `void_not_error` wrapping the call in
            // assertDoesNotThrow — nothing to render into assertions_body here.
        } else if is_streaming {
            out.push_str("        assertNotNull(chunks, \"expected drained chunks list\");\n");
        } else if not_error_may_assert_presence {
            out.push_str(&format!(
                "        assertNotNull({result_var}, \"expected non-null response\");\n"
            ));
        }
        return true;
    }
    false
}
