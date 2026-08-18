//! Error assertion rendering for generated Python tests.

use std::fmt::Write as FmtWrite;

use crate::e2e::escape::escape_python;
use crate::e2e::fixture::Fixture;

pub(super) fn emit_error_assertion(
    out: &mut String,
    fixture: &Fixture,
    arg_bindings_str: &str,
    call_expr: &str,
    is_streaming_error_call: bool,
) {
    let error_assertion = fixture.assertions.iter().find(|a| a.assertion_type == "error");
    let has_message = error_assertion
        .and_then(|a| a.value.as_ref())
        .and_then(|v| v.as_str())
        .is_some();

    render_unrenderable_error_path_assertions(out, fixture);

    // Re-indent arg_bindings by an extra 4 spaces so they land inside the `with`
    // block. arg_bindings already begin with 4 spaces (function-body level);
    // prepending 4 more puts them at the with-body level (8 spaces).
    let indented_bindings: String = arg_bindings_str
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("    {l}\n"))
        .collect();

    if has_message {
        let _ = writeln!(out, "    with pytest.raises(Exception) as exc_info:  # noqa: B017");
        out.push_str(&indented_bindings);
        if is_streaming_error_call {
            // The streaming iterator returns synchronously (chat_stream returns the
            // iterator without await); errors only appear when iterating via
            // __anext__. Strip the `await ` prefix the async-call codegen would
            // attach, then drain the iterator inside the raises block so the
            // exception propagates before the with-block exits.
            let sync_call_expr = call_expr.strip_prefix("await ").unwrap_or(call_expr);
            let _ = writeln!(out, "        _iterator = {sync_call_expr}");
            let _ = writeln!(out, "        async for _ in _iterator:");
            let _ = writeln!(out, "            pass");
        } else {
            let _ = writeln!(out, "        {call_expr}");
        }
        if let Some(msg) = error_assertion.and_then(|a| a.value.as_ref()).and_then(|v| v.as_str()) {
            let escaped = escape_python(msg);
            // Match against EITHER the rendered exception message OR the
            // exception class name. Different crates use different
            // fixture-shape conventions:
            //   * config-validation fixtures may use field names that are substrings
            //     of the user-facing error message, never of a class name.
            //   * API-error fixtures may use class-name prefixes such as
            //     `Authentication`, `BadRequest`, or `ContentPolicy`.
            //     `BadRequestError`, `ContentPolicyError`), not message text.
            // The disjunction lets a single codegen path satisfy both.
            let _ = writeln!(
                out,
                "    assert \"{escaped}\" in str(exc_info.value) or \"{escaped}\" in type(exc_info.value).__name__  # noqa: S101"
            );
        }
    } else {
        let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
        out.push_str(&indented_bindings);
        if is_streaming_error_call {
            let _ = writeln!(out, "        _iterator = {call_expr}");
            let _ = writeln!(out, "        async for _ in _iterator:");
            let _ = writeln!(out, "            pass");
        } else {
            let _ = writeln!(out, "        {call_expr}");
        }
    }
}

/// Every fixture assertion beyond the one `"error"`-type check [`emit_error_assertion`] renders
/// (a message-or-class-name match inside the `pytest.raises` block) used to be silently dropped: a
/// second `"error"` assertion, an `equals` against an `error.<field>` path, or any other assertion
/// type on an error-path fixture rendered nothing at all — not even a skip comment — because this
/// function returns before the fixture's other assertions are ever visited. `rust/assertions.rs`
/// is the only backend that resolves `error.<field>` paths (via `accessor_for_error`); every other
/// backend, python included, has no such accessor. Naming that gap here, rather than implementing
/// it, is the fix in scope: it gives `ALEF_E2E_STRICT_ASSERTIONS`
/// (`assertion_type_skip::AssertionTypeSkip::EqualsOnErrorFieldNotSupported`) a line to see instead
/// of a generated test that silently asserts nothing about the rest of the fixture. ~keep
fn render_unrenderable_error_path_assertions(out: &mut String, fixture: &Fixture) {
    let mut consumed_the_primary_error_check = false;
    for assertion in &fixture.assertions {
        if !consumed_the_primary_error_check && assertion.assertion_type == "error" {
            consumed_the_primary_error_check = true;
            continue;
        }
        let field = assertion.field.as_deref().unwrap_or("<none>");
        let _ = writeln!(
            out,
            "    # skipped: assertion type '{}' has no accessor for error field {field} in this backend",
            assertion.assertion_type
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_with_error(value: Option<serde_json::Value>) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "streaming_error".to_string(),
            description: "streaming error".to_string(),
            input: serde_json::Value::Null,
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![crate::e2e::fixture::Assertion {
                skip: None,
                assertion_type: "error".to_string(),
                field: None,
                value,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            call: None,
            skip: None,
            env: None,
            setup: Vec::new(),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn streaming_error_assertion_drains_iterator_inside_raises() {
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let mut out = String::new();

        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "await client.chat_stream(payload)",
            true,
        );

        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(out.contains("        payload = {}"), "got: {out}");
        assert!(
            out.contains("        _iterator = client.chat_stream(payload)"),
            "got: {out}"
        );
        assert!(out.contains("        async for _ in _iterator:"), "got: {out}");
        assert!(out.contains("BadRequest"), "got: {out}");
    }

    #[test]
    fn plain_error_assertion_emits_call_inside_raises() {
        let fixture = fixture_with_error(None);
        let mut out = String::new();

        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
        );

        assert!(out.contains("with pytest.raises(Exception):"), "got: {out}");
        assert!(out.contains("        payload = {}"), "got: {out}");
        assert!(out.contains("        client.create(payload)"), "got: {out}");
        assert!(!out.contains("async for _ in _iterator"), "got: {out}");
    }

    fn assertion(
        assertion_type: &str,
        field: Option<&str>,
        value: Option<serde_json::Value>,
    ) -> crate::e2e::fixture::Assertion {
        crate::e2e::fixture::Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|f| f.to_string()),
            value,
            ..crate::e2e::fixture::Assertion::default()
        }
    }

    fn fixture_with_assertions(assertions: Vec<crate::e2e::fixture::Assertion>) -> Fixture {
        Fixture {
            assertions,
            ..fixture_with_error(None)
        }
    }

    /// Drives the real emission path (not a hand-built `SkipRecord`): a fixture with an `equals`
    /// assertion against `error.status_code` alongside the primary `error` check. Before this
    /// change, `emit_error_assertion` rendered only the primary check and the second assertion
    /// left no trace in the output at all — the gate had nothing to scan. This proves three
    /// things in sequence: the primary check still actually runs, the second assertion is now
    /// named in a skip comment instead of vanishing, and the widened gate recognises exactly that
    /// comment as an assertion-type skip.
    #[test]
    fn equals_on_error_field_is_now_visible_and_counted_by_the_gate() {
        let fixture = fixture_with_assertions(vec![
            assertion("error", None, Some(serde_json::Value::String("BadRequest".to_string()))),
            assertion("equals", Some("error.status_code"), Some(serde_json::Value::from(429))),
        ]);
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
        );

        // The fixture's only assertion this backend can actually run must still run.
        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(
            out.contains(
                "assert \"BadRequest\" in str(exc_info.value) or \"BadRequest\" in type(exc_info.value).__name__"
            ),
            "the primary error assertion must still render: got: {out}"
        );

        // The second assertion must now be named, not silently dropped.
        assert!(
            out.contains(
                "# skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "got: {out}"
        );

        // And the widened gate must recognise exactly that line.
        let _ = crate::e2e::codegen::take_skip_records();
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out, "python", &fixture.id);
        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].field, "equals");
        assert_eq!(
            records[0].verdict,
            crate::e2e::codegen::SkipVerdict::AwaitingGeneratorSupport
        );
        assert_eq!(records[0].origin, crate::e2e::codegen::SkipOrigin::AssertionType);
    }

    /// Negative control: the fixture's one assertion IS rendered (the primary error check), so the
    /// assertion-type gate must find nothing to count. Without this, a gate that fires on every
    /// line would be exactly as uninformative as the gate that fired on none before this change.
    #[test]
    fn a_rendered_error_assertion_does_not_trip_the_assertion_type_gate() {
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
        );
        assert!(
            out.contains("assert \"BadRequest\" in str(exc_info.value)"),
            "the fixture's only assertion must actually render before we assert nothing was \
             flagged: got: {out}"
        );

        let _ = crate::e2e::codegen::take_skip_records();
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out, "python", &fixture.id);
        assert!(
            crate::e2e::codegen::take_skip_records().is_empty(),
            "a rendered assertion must not be recognised as an assertion-type skip"
        );
    }
}
