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
    errors: &[crate::core::ir::ErrorDef],
) {
    // ~keep Routed through the shared `declared_error_value` (see its own doc comment) rather
    // than a local `.find(|a| a.assertion_type == "error")`: a fixture commonly declares two
    // `"error"` assertions — a bare one, then one carrying the message/type-name value — and
    // only the shared helper looks past the first to find the one that actually has a value.
    let declared_value = crate::e2e::codegen::declared_error_value(fixture);
    let has_message = declared_value.is_some();
    // ~keep Reuses the same seam the docs-snippet renderer already consults
    // (`python/snippet.rs`) instead of re-deriving "does this fixture name a real variant" —
    // see the `two-generators-disagree` skill. `pyo3::create_exception!` gives every
    // `ErrorVariant` its own exception class unconditionally
    // (`declared_error_variant::substantiates_variant_identity`'s `"python" => true` arm), so
    // when the declared value names a real variant, `pytest.raises(<TheVariantError>)` is a
    // strictly stronger, type-discriminating check than the message-or-class-name substring
    // match below — it fails if the wrong error type is raised for any reason. The substring
    // fallback still renders for message-style values (config-validation fixtures whose
    // declared value is a message substring, not a variant name), which no per-variant class
    // exists for.
    let typed_branch = crate::e2e::codegen::snippet_error_branch::for_fixture("python", fixture, errors);

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
        if let Some(branch) = &typed_branch {
            // The fixture names a real `ErrorVariant` and pyo3 generates a dedicated exception
            // class for it — catch that class directly. No `# noqa: B017` needed: B017 warns
            // specifically about the broad `pytest.raises(Exception)`, and a named class is
            // exactly the narrower catch the lint wants. Unlike the substring fallback below,
            // this fails the test when the wrong error type is raised, even if its message or
            // class name happens to contain the same substring.
            let _ = writeln!(out, "    with pytest.raises({}):", branch.host_type);
        } else {
            let _ = writeln!(out, "    with pytest.raises(Exception) as exc_info:  # noqa: B017");
        }
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
        if typed_branch.is_none()
            && let Some(msg) = declared_value
        {
            let escaped = escape_python(msg);
            // Match against EITHER the rendered exception message OR the
            // exception class name. Different crates use different
            // fixture-shape conventions:
            //   * config-validation fixtures may use field names that are substrings
            //     of the user-facing error message, never of a class name.
            //   * API-error fixtures may use class-name prefixes such as
            //     `Authentication`, `BadRequest`, or `ContentPolicy`.
            //     `BadRequestError`, `ContentPolicyError`), not message text.
            // The disjunction lets a single codegen path satisfy both. Only reached when no
            // typed class exists for the declared value (see `typed_branch` above).
            let _ = writeln!(
                out,
                "    assert \"{escaped}\" in str(exc_info.value) or \"{escaped}\" in type(exc_info.value).__name__"
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
/// function returns before the fixture's other assertions are ever visited. The wording, the
/// ledger recording and the reason no non-`rust` backend can resolve `error.<field>` now all live
/// in [`crate::e2e::codegen::error_path_assertions`], shared with every other backend's error
/// block; this stays as the python-shaped entry point (comment token `#`, four-space indent). ~keep
fn render_unrenderable_error_path_assertions(out: &mut String, fixture: &Fixture) {
    crate::e2e::codegen::error_path_assertions::emit(out, fixture, "    # ", "python");
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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

    /// The exact shape observed live in `crawlberg`'s `validation_ssrf_*` fixtures: a bare
    /// `{"type": "error"}` assertion FOLLOWED BY `{"type": "error", "value": "..."}`. Before the
    /// fix, `emit_error_assertion` found only the first (bare) `"error"` assertion, so
    /// `has_message` was always false for this shape and the generated test dropped the message
    /// check entirely — `with pytest.raises(Exception):` with no `assert "..." in ...` line, so
    /// `assert result.is_err()` could not tell an SSRF refusal from an unrelated failure. This
    /// must render the message check exactly as it would if the fixture had declared the value on
    /// its only `"error"` assertion.
    #[test]
    fn a_bare_check_followed_by_a_valued_one_still_renders_the_message_check() {
        let fixture = fixture_with_assertions(vec![
            assertion("error", None, None),
            assertion(
                "error",
                None,
                Some(serde_json::Value::String("ssrf_policy_violation".to_string())),
            ),
        ]);
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    url = \"http://127.0.0.1:9/\"\n",
            "scrape(engine, url)",
            false,
            &[],
        );

        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(
            out.contains(
                "assert \"ssrf_policy_violation\" in str(exc_info.value) or \"ssrf_policy_violation\" in \
                 type(exc_info.value).__name__"
            ),
            "the declared value on the second `error` assertion must still render a message \
             check: got: {out}"
        );
    }

    fn error_def_with_variant(error_name: &str, variant_name: &str) -> crate::core::ir::ErrorDef {
        crate::core::ir::ErrorDef {
            name: error_name.to_string(),
            rust_path: format!("lib::{error_name}"),
            original_rust_path: String::new(),
            variants: vec![crate::core::ir::ErrorVariant {
                name: variant_name.to_string(),
                is_unit: true,
                ..crate::core::ir::ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// The structural half of the xberg #1525 fix: when a fixture's declared `error` value
    /// names a real `ErrorVariant`, `pyo3::create_exception!` gives it a dedicated exception
    /// class unconditionally (`declared_error_variant::substantiates_variant_identity`'s
    /// `"python" => true` arm), so the generated assertion must catch THAT class rather than
    /// the broad `Exception`, and the substring proxy the class-scoped catch supersedes must
    /// not also render.
    #[test]
    fn a_declared_variant_renders_a_class_scoped_raises_instead_of_the_substring_proxy() {
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let errors = vec![error_def_with_variant("ApiError", "BadRequest")];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
        );

        assert!(out.contains("with pytest.raises(BadRequestError):"), "got: {out}");
        assert!(!out.contains("pytest.raises(Exception)"), "got: {out}");
        assert!(
            !out.contains("in str(exc_info.value) or"),
            "the class-scoped catch makes the substring proxy redundant: got: {out}"
        );
    }

    fn python3_available() -> bool {
        which::which("python3").is_ok()
    }

    /// A minimal `pytest.raises` stand-in carrying the ONE behaviour this test cares about:
    /// like real `pytest.raises`, it does NOT suppress an exception whose type is not a
    /// subclass of the expected one — it propagates, failing the enclosing test. There is no
    /// `pytest` package dependency available to a Rust unit test, so this mirrors just that
    /// discriminating behaviour rather than pulling one in. ~keep
    const PYTEST_RAISES_STUB: &str = "\
class _Raises:
    def __init__(self, expected):
        self.expected = expected

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, tb):
        if exc_type is None:
            raise AssertionError(f\"DID NOT RAISE {self.expected}\")
        return issubclass(exc_type, self.expected)


def raises(expected, *args, **kwargs):
    return _Raises(expected)
";

    /// Runs `raises_block` (the exact text [`emit_error_assertion`] renders for the `with
    /// pytest.raises(...)` block) as a real Python 3 process under [`PYTEST_RAISES_STUB`],
    /// with `BadRequestError`/`UnrelatedError` classes defined and a `client.create(...)` that
    /// raises `raising_class`. Returns whether the script ran to completion with no uncaught
    /// exception — i.e. whether the generated assertion would have passed.
    fn generated_assertion_passes_when_call_raises(raises_block: &str, raising_class: &str) -> bool {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("pytest.py"), PYTEST_RAISES_STUB).expect("write pytest stub");
        let script = format!(
            "import pytest\n\n\
             class BadRequestError(Exception):\n    pass\n\n\
             class UnrelatedError(Exception):\n    pass\n\n\
             class _Client:\n    def create(self, payload):\n        raise {raising_class}(\"BadRequest-shaped input rejected\")\n\n\
             client = _Client()\n\n\
             def test_case():\n{raises_block}\n\
             test_case()\n"
        );
        std::fs::write(dir.path().join("script.py"), script).expect("write script");
        let status = std::process::Command::new("python3")
            .arg("script.py")
            .current_dir(dir.path())
            .status()
            .expect("run python3");
        status.success()
    }

    /// The runtime half of the xberg #1525 fix, and the one property the replaced substring
    /// proxy provably lacked: it passed for ANY exception whose message or class name merely
    /// *contained* the declared variant name — `"BadRequest" in str(exc_info.value) or ...` —
    /// including an unrelated error. `UnrelatedError("BadRequest-shaped input rejected")` is
    /// exactly that shape: its message contains the substring, its class does not carry the
    /// name. Under the class-scoped `pytest.raises(BadRequestError)` this now renders, that
    /// call must FAIL the generated assertion — proving the discrimination the substring check
    /// could never provide — while the real `BadRequestError` must still pass it.
    #[test]
    fn wrong_error_type_fails_the_generated_assertion_even_when_its_message_matches() {
        if !python3_available() {
            return;
        }
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let errors = vec![error_def_with_variant("ApiError", "BadRequest")];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
        );
        let raises_block = out.trim_start_matches('\n');

        assert!(
            generated_assertion_passes_when_call_raises(raises_block, "BadRequestError"),
            "the correct error type must satisfy the generated assertion"
        );
        assert!(
            !generated_assertion_passes_when_call_raises(raises_block, "UnrelatedError"),
            "an unrelated error type whose MESSAGE merely contains the variant name must fail \
             the generated assertion, not pass it — this is exactly what the substring proxy \
             could not do"
        );
    }
}
