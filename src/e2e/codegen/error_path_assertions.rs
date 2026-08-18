//! The one place every backend names the assertions its error block cannot render.
//!
//! ~keep Every backend's error path is shaped the same way: it locates the fixture's one
//! `"error"`-type assertion (`assertions.iter().find(..)` / `.any(..)`), renders a
//! "the call must fail" check plus — where the backend implements it — a message match, and then
//! **returns**. Every other assertion on that fixture is never visited by any rendering code, so
//! it produces no output at all: not an assertion, not a skip comment, nothing for
//! `ALEF_E2E_STRICT_ASSERTIONS` to see. A silently dropped assertion is indistinguishable from a
//! fixture that never declared one.
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
//! The rendered wording is the one
//! [`super::assertion_type_skip::AssertionTypeSkip::EqualsOnErrorFieldNotSupported`] recognises, so
//! every marker this module writes is counted by
//! [`super::fail_on_unsupported_assertion_type_markers`] — which [`render`] calls itself, so a
//! backend cannot wire the wording in and forget the gate.

use std::fmt::Write as FmtWrite;

use crate::e2e::fixture::Fixture;

/// Render one skip marker per fixture assertion the backend's error block does not render, and
/// record each one on the shared skip ledger.
///
/// `line_prefix` is the full leading text for a marker line — indentation plus the backend's
/// comment token, e.g. `"    // "`, `"\t// "` or `"    # "`. The first `"error"`-type assertion is
/// the one every error block *does* render, so it is never marked; everything after it is.
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
    let mut consumed_the_primary_error_check = false;
    for assertion in &fixture.assertions {
        if !consumed_the_primary_error_check && assertion.assertion_type == "error" {
            consumed_the_primary_error_check = true;
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

    /// Only the *first* `error` assertion is the one a backend's error block renders — a second
    /// one is dropped exactly like any other trailing assertion and must be named.
    #[test]
    fn a_second_error_assertion_is_also_named() {
        let fixture = fixture_with(vec![assertion("error", None), assertion("error", None)]);
        let _ = crate::e2e::codegen::take_skip_records();
        let rendered = render(&fixture, "    # ", "ruby");

        assert_eq!(
            rendered,
            "    # skipped: assertion type 'error' has no accessor for error field <none> in this backend\n"
        );
        assert_eq!(crate::e2e::codegen::take_skip_records().len(), 1);
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
