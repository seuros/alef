//! Result and streaming assertion rendering for generated Python tests.

use std::fmt::Write as FmtWrite;

use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::super::assertions::render_assertion;
use super::super::helpers::resolve_assert_enum_fields;
use super::super::json::value_to_python_string;

/// True when `body` contains at least one line that is not blank and not a
/// `#`-prefixed comment — i.e. an executable `assert` statement. A body made
/// up only of "# skipped: ..." comments is not executable.
fn has_real_assertion(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#')
    })
}

/// When a fixture declares at least one assertion but the rendered body has
/// no executable statement — its only assertion was `not_error`, or every
/// field assertion resolved to a "skipped" comment — inject a real assertion
/// on the result instead of leaving the test vacuous. Fixtures that declare
/// NO assertions at all are left untouched: that's a pre-existing, intentional
/// "just call it" smoke test contract (see
/// `should_discard_result_when_force_bind_result_is_unset_and_unused`). ~keep
fn apply_vacuous_assertion_fallback(temp_assertions: &mut String, has_declared_assertions: bool, result_var: &str) {
    if !has_declared_assertions || has_real_assertion(temp_assertions) {
        return;
    }
    let _ = writeln!(temp_assertions, "    assert {result_var} is not None  # noqa: S101");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_result_and_assertions(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    call_config: &crate::e2e::config::CallConfig,
    call_expr: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    is_streaming: bool,
    force_bind_result: bool,
) {
    // Streaming virtual fields resolve against the collected `chunks` list, not
    // the result type.
    //
    // ~keep This used to be preceded by a `let _ = fixture.assertions.iter()
    // .any(...)` closure computing a `has_usable_assertion`-shaped predicate
    // (excluding not_error/error, accepting streaming-virtual and
    // result_is_simple fields) whose result was discarded (`let _ =`) and never
    // referenced anywhere in this function — dead code that looked like a check.
    // The real usability decision lives below, derived from what
    // `apply_vacuous_assertion_fallback`/`temp_assertions` actually render, not
    // a separately maintained predicate that can drift out of sync with it (see
    // the php/typescript/ruby fixes for the same drift in this defect class).
    let chunks_var = "chunks";

    let fields_enum = e2e_config.effective_fields_enum(call_config);
    let assert_enum_fields = resolve_assert_enum_fields(call_config);

    // For streaming fixtures: bind the raw iterator, then drain it into a list.
    // The Python ChatStreamIterator exposes __aiter__/__anext__ (async iterator),
    // so the test function must be `async def` and we use `async for` to drain.
    // Note: chat_stream() itself is NOT a coroutine in Python — it returns the
    // iterator synchronously (blocking on stream acquisition via block_on), so
    // no `await` prefix is used on the call expression.
    if is_streaming {
        let _ = writeln!(out, "    {result_var} = {call_expr}");
        if let Some(collect) = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            "python", result_var, chunks_var,
        ) {
            let _ = writeln!(out, "    {collect}");
        }
        // Render streaming assertions using the collected chunks
        for assertion in &fixture.assertions {
            if assertion.assertion_type == "not_error" || assertion.assertion_type == "error" {
                continue;
            }
            if let Some(f) = &assertion.field
                && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
            {
                emit_streaming_virtual_assertion(out, assertion, f, chunks_var);
                continue;
            }
            // Non-streaming-virtual assertions on streaming fixtures are skipped
            // (the result type doesn't have these fields during iteration).
        }
    } else {
        // For non-streaming: render assertions to a temporary buffer first,
        // then check if result_var is referenced. Only emit the assignment if it is.
        let mut temp_assertions = String::new();

        for assertion in &fixture.assertions {
            // `not_error` has no explicit rendering: an uncaught exception already
            // fails the test, so the check is implicit in the call succeeding.
            if assertion.assertion_type == "not_error" {
                continue;
            }
            render_assertion(
                &mut temp_assertions,
                assertion,
                result_var,
                field_resolver,
                fields_enum,
                assert_enum_fields,
                result_is_simple,
            );
        }

        apply_vacuous_assertion_fallback(&mut temp_assertions, !fixture.assertions.is_empty(), result_var);

        // Check if result_var appears in actual code (not in comments).
        // Only count lines that start with "assert" or contain actual code tokens.
        // Comments (lines starting with #) are skipped to avoid false positives
        // from strings like "field 'result' not available" in comment text.
        let result_var_used = temp_assertions.lines().any(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with('#') && trimmed.contains(result_var)
        });

        let result_binding =
            (result_var_used || fixture.has_docs_presentation() || force_bind_result).then_some(result_var);
        out.push_str(&crate::e2e::template_env::render(
            "python/call_statement.py.jinja",
            minijinja::context! { result_binding => result_binding, call_expr => call_expr },
        ));
        out.push_str(&temp_assertions);
    }
}

/// Emit a Python assertion for a streaming virtual field using the collected
/// `chunks` list.  Mirrors the pattern in rust/assertions.rs.
fn emit_streaming_virtual_assertion(out: &mut String, assertion: &Assertion, field: &str, chunks_var: &str) {
    use crate::e2e::codegen::streaming_assertions::StreamingFieldResolver;

    let Some(expr) = StreamingFieldResolver::accessor(field, "python", chunks_var) else {
        let _ = writeln!(out, "    # skipped: streaming field '{field}': no python accessor");
        return;
    };

    match assertion.assertion_type.as_str() {
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({expr}) >= {n}  # noqa: S101");
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({expr}) == {n}  # noqa: S101");
            }
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                if val.is_string() {
                    let _ = writeln!(out, "    assert {expr}.strip() {op} {expected}.strip()  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {expr} {op} {expected}  # noqa: S101");
                }
            }
        }
        "not_empty" => {
            // Bare truthiness would reject a legitimate 0/0.0/False. Only sized values
            // carry an emptiness notion; everything else just has to be present.
            let _ = writeln!(
                out,
                "    assert {expr} is not None and (not hasattr({expr}, \"__len__\") or len({expr}) > 0)  # noqa: S101"
            );
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert not {expr}  # noqa: S101");
        }
        "is_true" => {
            // Normalize "true"/"false" literals to Python's True/False.
            let py_expr = if expr == "true" {
                "True".to_string()
            } else if expr == "false" {
                "False".to_string()
            } else {
                expr.clone()
            };
            let _ = writeln!(out, "    assert {py_expr}  # noqa: S101");
        }
        "is_false" => {
            let py_expr = if expr == "true" {
                "True".to_string()
            } else if expr == "false" {
                "False".to_string()
            } else {
                expr.clone()
            };
            let _ = writeln!(out, "    assert not {py_expr}  # noqa: S101");
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} > {expected}  # noqa: S101");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} >= {expected}  # noqa: S101");
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expected} in {expr}  # noqa: S101");
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "    # skipped: streaming field '{field}': assertion type '{}' not rendered",
                assertion.assertion_type
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(str::to_string),
            value,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }
    }

    fn minimal_fixture() -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "widget_smoke".to_string(),
            description: "Create a widget".to_string(),
            input: serde_json::Value::Null,
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            setup: Vec::new(),
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn streaming_virtual_assertion_renders_collected_chunks_access() {
        let mut out = String::new();
        let assertion = assertion("count_min", Some("chunks"), Some(serde_json::Value::from(1)));

        emit_streaming_virtual_assertion(&mut out, &assertion, "chunks", "chunks");

        assert!(out.contains("assert len(chunks) >= 1"), "got: {out}");
    }

    #[test]
    fn not_empty_for_python_streaming_rejects_empty_chunks_but_accepts_zero() {
        let mut out = String::new();
        let assertion = assertion("not_empty", Some("chunks"), None);

        emit_streaming_virtual_assertion(&mut out, &assertion, "chunks", "chunks");

        // Bare `assert chunks` fails on a legitimate 0, 0.0 or False.
        assert_eq!(
            out.trim(),
            "assert chunks is not None and (not hasattr(chunks, \"__len__\") or len(chunks) > 0)  # noqa: S101"
        );
    }

    #[test]
    fn call_statement_omits_binding_when_the_result_is_unused() {
        let rendered = crate::e2e::template_env::render(
            "python/call_statement.py.jinja",
            minijinja::context! { result_binding => Option::<&str>::None, call_expr => "await process(value)" },
        );

        assert_eq!(rendered, "    await process(value)\n");
        assert!(!rendered.contains("result ="));
        assert!(!rendered.contains("_ ="));
    }

    #[test]
    fn should_bind_result_when_force_bind_result_is_set_with_no_assertions() {
        let fixture = minimal_fixture();
        let e2e_config = E2eConfig::default();
        let call_config = crate::e2e::config::CallConfig::default();
        let field_resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        let mut out = String::new();

        emit_result_and_assertions(
            &mut out,
            &fixture,
            &e2e_config,
            &call_config,
            "await widget_client.create()",
            "result",
            &field_resolver,
            false,
            false,
            true,
        );

        assert!(
            out.contains("result = await widget_client.create()"),
            "expected the call result to be bound so a caller can print it, got: {out}"
        );
    }

    #[test]
    fn should_discard_result_when_force_bind_result_is_unset_and_unused() {
        let fixture = minimal_fixture();
        let e2e_config = E2eConfig::default();
        let call_config = crate::e2e::config::CallConfig::default();
        let field_resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        let mut out = String::new();

        emit_result_and_assertions(
            &mut out,
            &fixture,
            &e2e_config,
            &call_config,
            "await widget_client.create()",
            "result",
            &field_resolver,
            false,
            false,
            false,
        );

        assert!(!out.contains("result ="), "unused result must not be bound, got: {out}");
    }

    #[test]
    fn has_real_assertion_is_false_for_comment_only_body() {
        let body = "    # skipped: field 'foo' not available on result type\n";
        assert!(
            !has_real_assertion(body),
            "comment-only body must not count as asserting"
        );
    }

    #[test]
    fn has_real_assertion_is_true_when_a_real_statement_is_present() {
        let body = "    # skipped: field 'foo' not available on result type\n    assert result.ok\n";
        assert!(has_real_assertion(body), "a real assert line must count as asserting");
    }

    #[test]
    fn vacuous_fallback_is_a_noop_without_declared_assertions() {
        let mut body = String::new();
        apply_vacuous_assertion_fallback(&mut body, false, "result");
        assert!(
            body.is_empty(),
            "a fixture with no declared assertions is an intentional smoke test and must stay untouched"
        );
    }

    #[test]
    fn vacuous_fallback_emits_a_real_assertion_when_body_is_empty() {
        let mut body = String::new();
        apply_vacuous_assertion_fallback(&mut body, true, "result");
        assert_eq!(body, "    assert result is not None  # noqa: S101\n");
    }

    #[test]
    fn vacuous_fallback_emits_a_real_assertion_over_comment_only_body() {
        let mut body = "    # skipped: field 'chunks' not available on result type\n".to_string();
        apply_vacuous_assertion_fallback(&mut body, true, "result");
        assert!(
            body.contains("assert result is not None"),
            "a comment-only body must still get a real fallback assertion, got: {body}"
        );
    }

    #[test]
    fn vacuous_fallback_leaves_a_real_assertion_untouched() {
        let mut body = "    assert result.count == 1  # noqa: S101\n".to_string();
        let original = body.clone();
        apply_vacuous_assertion_fallback(&mut body, true, "result");
        assert_eq!(
            body, original,
            "a fixture with a real assertion must not get an extra fallback line"
        );
    }

    /// Regression test for the not_error-only vacuous-test defect: a fixture whose
    /// only declared assertion is `not_error` must bind the call result and emit a
    /// real assertion, not silently discard the result with no assertion at all.
    #[test]
    fn not_error_only_fixture_binds_result_and_emits_real_assertion() {
        let mut fixture = minimal_fixture();
        fixture.assertions = vec![assertion("not_error", None, None)];
        let e2e_config = E2eConfig::default();
        let call_config = crate::e2e::config::CallConfig::default();
        let field_resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        let mut out = String::new();

        emit_result_and_assertions(
            &mut out,
            &fixture,
            &e2e_config,
            &call_config,
            "await widget_client.create()",
            "result",
            &field_resolver,
            false,
            false,
            false,
        );

        assert!(
            out.contains("result = await widget_client.create()"),
            "a not_error-only fixture must bind the result, got: {out}"
        );
        assert!(
            out.contains("assert result is not None"),
            "a not_error-only fixture must emit a real assertion instead of a vacuous body, got: {out}"
        );
    }
}
