//! Regression coverage for TypeScript's skip-marker rendering on unavailable/unsupported
//! synthetic-field assertions.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::render_synthetic_field_assertion;
use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::{SkipVerdict, fail_on_unavailable_field_markers, take_skip_records};
use crate::e2e::fixture::Assertion;

fn render_streaming(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> (String, bool) {
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
        ..Assertion::default()
    };
    let mut out = String::new();
    let handled = render_synthetic_field_assertion(&mut out, &assertion, "result", field, true);
    (out, handled)
}

fn field_verdicts(body: &str) -> Vec<SkipVerdict> {
    let _ = take_skip_records();
    fail_on_unavailable_field_markers(body, "node", "stream_smoke", &[]);
    take_skip_records().into_iter().map(|record| record.verdict).collect()
}

/// Non-vacuity control: the same harness on a resolvable streaming field must render a real
/// `expect(...)`, or the marker assertions below would be facts about the harness. ~keep
#[test]
fn the_streaming_harness_renders_a_real_expectation_when_the_accessor_resolves() {
    let (out, handled) = render_streaming("count_min", "chunks", Some(serde_json::json!(2)));
    assert!(handled, "the streaming arm must claim the assertion");
    assert!(
        out.contains("expect(chunks.length).toBeGreaterThanOrEqual(2)"),
        "got: {out}"
    );
    assert!(
        field_verdicts(&out).is_empty(),
        "a live assertion records no skip: {out}"
    );
}

/// The arm returned `true` — "handled" — while writing nothing, so the assertion vanished and
/// no later branch could rescue it. `wasm` shares this renderer and has no streaming at all,
/// so this is the whole-language case, not an edge one. ~keep
#[test]
fn a_streaming_field_with_no_accessor_emits_a_counted_marker() {
    let (out, handled) = render_streaming("is_true", "stream.has_page_event", None);
    assert!(handled, "the streaming arm still claims the assertion");
    assert!(!out.is_empty(), "the assertion must not vanish");
    assert_eq!(
        FieldSkip::extract_classified(out.trim_end()),
        Some(("stream.has_page_event", FieldSkip::StreamingAssertionOnUnsupportedField)),
        "got: {out}"
    );
    assert_eq!(field_verdicts(&out), vec![SkipVerdict::AwaitingGeneratorSupport]);
}

#[test]
fn an_unrenderable_streaming_assertion_type_emits_a_registered_marker() {
    let (out, _) = render_streaming("matches_regex", "chunks", None);
    assert_eq!(
        AssertionTypeSkip::extract_classified(out.trim_end()),
        Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
        "got: {out}"
    );
}

#[test]
fn a_streaming_assertion_with_no_value_emits_a_registered_marker() {
    let (out, _) = render_streaming("count_min", "chunks", None);
    assert_eq!(
        AssertionTypeSkip::extract_classified(out.trim_end()),
        Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
        "got: {out}"
    );
}
