//! The one place every backend names the assertions its error block cannot render.
//!
//! ~keep Every backend's error path is shaped the same way: it locates the fixture's declared
//! `"error"` value (via [`super::declared_error_value`]), renders a "the call must fail" check
//! plus — where the backend implements it — a message match, and then **returns**. Every other
//! assertion on that fixture is never visited by any rendering code, so it produces no output at
//! all: not an assertion, not a skip comment, nothing for `ALEF_E2E_STRICT_ASSERTIONS` to see. A
//! silently dropped assertion is indistinguishable from a fixture that never declared one.
//!
//! The commonest dropped shape is `equals` against an `error.<field>` path (e.g.
//! `error.status_code`). Only the `rust` backend resolves those, via
//! `FieldResolver::accessor_for_error`, and it can do so because
//! `[e2e.error_field_aliases]` is documented as mapping to fields on the **Rust** error type.
//! No binding re-exposes those fields: `pyo3::create_exception!`, the JNI/P-Invoke/cgo exception
//! and error mappings and the C ABI (`{prefix}_last_error_code` / `{prefix}_last_error_context`)
//! all carry a message and, at most, a numeric FFI-taxonomy code — never the error struct's own
//! fields. Naming the gap is therefore the honest fix for every non-`rust` backend; implementing
//! an accessor would mean inventing a field the binding does not have.
//!
//! ~keep A DIFFERENT shape used to be conflated with that one and must not be again: fixture
//! authors routinely declare a fixture's error expectation as two `"error"`-type assertions — a
//! bare `{"type": "error"}` ("the call must fail") followed by `{"type": "error", "value": "..."}`
//! ("...with this message/type-name"). Every one of those `"error"` assertions IS fully rendered
//! — the bare one by the universal must-fail check, the valued one by
//! [`super::declared_error_value`]'s message-or-type-name check — so neither may be marked as an
//! unrenderable field access. Only a *second declared value* (a shape no observed fixture uses)
//! is genuinely unrenderable, because every backend's message check only ever consults the first
//! one; see [`AssertionTypeSkip::AdditionalDeclaredErrorValueNotChecked`] below.
//!
//! The rendered wording is the one
//! [`super::assertion_type_skip::AssertionTypeSkip::EqualsOnErrorFieldNotSupported`] recognises, so
//! every marker this module writes is counted by
//! [`super::fail_on_unsupported_assertion_type_markers`] — which [`render`] calls itself, so a
//! backend cannot wire the wording in and forget the gate.

use std::fmt::Write as FmtWrite;

use crate::e2e::fixture::Fixture;

/// Whether an `"error"`-type assertion carries a message/type-name value, i.e. is the shape
/// [`super::declared_error_value`] looks for rather than the bare "must fail" shape.
fn is_valued_error_assertion(assertion: &crate::e2e::fixture::Assertion) -> bool {
    assertion.assertion_type == "error" && assertion.value.as_ref().and_then(serde_json::Value::as_str).is_some()
}

/// Render one skip marker per fixture assertion the backend's error block does not render, and
/// record each one on the shared skip ledger.
///
/// `line_prefix` is the full leading text for a marker line — indentation plus the backend's
/// comment token, e.g. `"    // "`, `"\t// "` or `"    # "`. Every bare `"error"` assertion (no
/// declared value) is rendered by the universal must-fail check, and the first `"error"`
/// assertion carrying a value is rendered by [`super::declared_error_value`]'s message check —
/// neither is ever marked. Anything after that (a non-`error` assertion type, or a second
/// declared error value) is.
///
/// Returns an empty string for the overwhelmingly common single-`error`-assertion fixture, so a
/// backend that adopts this helper leaves those fixtures' output byte-identical.
pub(crate) fn render(fixture: &Fixture, line_prefix: &str, language: &str) -> String {
    // ~keep A fixture with no `"error"` assertion never reaches a backend's error block, and its
    // assertions are rendered normally by the happy path. Guarding here rather than at each of the
    // ~20 call sites means a call site placed outside its `expects_error` branch degrades to a
    // no-op instead of marking every assertion in the suite as unrenderable.
    if !fixture.assertions.iter().any(|a| a.assertion_type == "error") {
        return String::new();
    }
    let mut out = String::new();
    let mut consumed_the_declared_value = false;
    for assertion in &fixture.assertions {
        if assertion.assertion_type == "error" {
            if !is_valued_error_assertion(assertion) {
                // The bare "the call must fail" check every backend already renders, no matter
                // how many bare `"error"` assertions a fixture repeats.
                continue;
            }
            if !consumed_the_declared_value {
                consumed_the_declared_value = true;
                // Rendered by `declared_error_value`'s message-or-type-name check.
                continue;
            }
            let _ = writeln!(
                out,
                "{line_prefix}skipped: {}",
                super::assertion_type_skip::AssertionTypeSkip::AdditionalDeclaredErrorValueNotChecked.message("error")
            );
            continue;
        }
        let field = assertion.field.as_deref().unwrap_or("<none>");
        let _ = writeln!(
            out,
            "{line_prefix}skipped: assertion type '{}' has no accessor for error field {field} in this backend",
            assertion.assertion_type
        );
    }
    super::fail_on_unsupported_assertion_type_markers(&out, language, &fixture.id);
    out
}

/// [`render`], appended straight onto a backend's output buffer.
pub(crate) fn emit(out: &mut String, fixture: &Fixture, line_prefix: &str, language: &str) {
    out.push_str(&render(fixture, line_prefix, language));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::fixture::Assertion;

    fn assertion(assertion_type: &str, field: Option<&str>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(str::to_string),
            ..Assertion::default()
        }
    }

    fn error_assertion_with_value(value: &str) -> Assertion {
        Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }
    }

    fn fixture_with(assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            id: "rate_limited".to_string(),
            assertions,
            ..Fixture::default()
        }
    }

    #[test]
    fn a_lone_error_assertion_renders_no_marker() {
        let fixture = fixture_with(vec![assertion("error", None)]);
        let _ = crate::e2e::codegen::take_skip_records();
        assert_eq!(render(&fixture, "    // ", "go"), "");
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    #[test]
    fn an_error_field_equals_assertion_is_named_and_counted() {
        let fixture = fixture_with(vec![
            assertion("error", None),
            assertion("equals", Some("error.status_code")),
        ]);
        let _ = crate::e2e::codegen::take_skip_records();
        let rendered = render(&fixture, "\t// ", "go");

        assert_eq!(
            rendered,
            "\t// skipped: assertion type 'equals' has no accessor for error field error.status_code in this \
             backend\n"
        );
        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].field, "equals");
        assert_eq!(records[0].language, "go");
        assert_eq!(records[0].fixture_id, "rate_limited");
        assert_eq!(records[0].origin, crate::e2e::codegen::SkipOrigin::AssertionType);
        assert_eq!(
            records[0].verdict,
            crate::e2e::codegen::SkipVerdict::AwaitingGeneratorSupport
        );
    }

    /// A fixture repeating the bare `"error"` shape twice declares nothing a single "must fail"
    /// check does not already cover, so neither occurrence is ever marked unrenderable.
    #[test]
    fn two_bare_error_assertions_render_no_marker() {
        let fixture = fixture_with(vec![assertion("error", None), assertion("error", None)]);
        let _ = crate::e2e::codegen::take_skip_records();
        assert_eq!(render(&fixture, "    # ", "ruby"), "");
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    /// The regression this module exists to fix: a fixture's real, observed shape is a bare
    /// `{"type": "error"}` FOLLOWED BY `{"type": "error", "value": "..."}` — the message/type-name
    /// requirement is declared second, not first. Before the fix, every backend selected the
    /// fixture's *first* `"error"` assertion for its message check (finding the bare one, and
    /// discarding the declared value), while this module marked the second, valued one as an
    /// unrenderable field access using `<none>` as the field name — actively hiding that the
    /// value existed. Neither must happen: the declared value is real and every backend can check
    /// it, so it must render silently, exactly like the single-`error`-assertion case.
    #[test]
    fn a_bare_check_followed_by_a_valued_one_renders_no_marker() {
        let fixture = fixture_with(vec![
            assertion("error", None),
            error_assertion_with_value("ssrf_policy_violation"),
        ]);
        assert_eq!(
            crate::e2e::codegen::declared_error_value(&fixture),
            Some("ssrf_policy_violation"),
            "the shared lookup must find the value on the second assertion, not just the first"
        );
        let _ = crate::e2e::codegen::take_skip_records();
        assert_eq!(render(&fixture, "    # ", "python"), "");
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    /// The declared-value convention also works when the valued assertion is written FIRST — the
    /// order fixture authors happen to use should never matter.
    #[test]
    fn a_valued_check_followed_by_a_bare_one_renders_no_marker() {
        let fixture = fixture_with(vec![
            error_assertion_with_value("Authentication"),
            assertion("error", None),
        ]);
        assert_eq!(
            crate::e2e::codegen::declared_error_value(&fixture),
            Some("Authentication")
        );
        let _ = crate::e2e::codegen::take_skip_records();
        assert_eq!(render(&fixture, "    # ", "go"), "");
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    /// A SECOND declared value is the genuinely unrenderable shape: every backend's message check
    /// only ever consults the first declared value, so a second one has no check to land in. This
    /// is named honestly (an extra declared value, not a missing field accessor) and still counted
    /// by the strict gate.
    #[test]
    fn a_second_declared_value_is_named_and_counted() {
        let fixture = fixture_with(vec![
            error_assertion_with_value("rate_limited"),
            error_assertion_with_value("429"),
        ]);
        let _ = crate::e2e::codegen::take_skip_records();
        let rendered = render(&fixture, "    # ", "ruby");

        assert_eq!(
            rendered,
            "    # skipped: assertion type 'error' has an additional declared value not checked by this \
             backend\n"
        );
        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].field, "error");
        assert_eq!(
            records[0].verdict,
            crate::e2e::codegen::SkipVerdict::AwaitingGeneratorSupport
        );
    }

    /// The guard that lets a call site sit outside its backend's `expects_error` branch without
    /// marking a whole happy-path fixture as unrenderable.
    #[test]
    fn a_fixture_with_no_error_assertion_renders_nothing() {
        let fixture = fixture_with(vec![
            assertion("equals", Some("status_code")),
            assertion("not_empty", Some("content")),
        ]);
        let _ = crate::e2e::codegen::take_skip_records();
        assert_eq!(render(&fixture, "    // ", "dart"), "");
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    #[test]
    fn every_trailing_assertion_gets_its_own_line() {
        let fixture = fixture_with(vec![
            assertion("error", None),
            assertion("equals", Some("error.status_code")),
            assertion("contains", Some("error.message")),
        ]);
        let _ = crate::e2e::codegen::take_skip_records();
        let rendered = render(&fixture, "    // ", "swift");

        assert_eq!(rendered.lines().count(), 2, "got: {rendered}");
        assert_eq!(crate::e2e::codegen::take_skip_records().len(), 2);
    }
}
