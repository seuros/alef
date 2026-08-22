//! Tests for the unsupported-assertion-type skip ledger in [`super`].
//!
//! Sibling of `unavailable_field_marker_tests`; see that file for why both were split out.

use super::{SkipOrigin, SkipVerdict, fail_on_unsupported_assertion_type_markers, skip_summary, take_skip_records};

fn verdicts_for(body: &str, language: &str) -> Vec<SkipVerdict> {
    let _ = take_skip_records();
    fail_on_unsupported_assertion_type_markers(body, language, "smoke");
    take_skip_records().into_iter().map(|r| r.verdict).collect()
}

/// Every record this gate produces must be tagged [`SkipOrigin::AssertionType`], never
/// [`SkipOrigin::Field`] — the two axes must stay distinguishable downstream.
#[test]
fn recorded_markers_are_tagged_with_the_assertion_type_origin() {
    let _ = take_skip_records();
    fail_on_unsupported_assertion_type_markers(
        "\t// skipped: unsupported assertion type on synthetic field 'embeddings'\n",
        "go",
        "smoke",
    );
    let records = take_skip_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].origin, SkipOrigin::AssertionType);
}

/// GeneratorGap-classified wordings (alef's own debt) never fail a build, mirroring the field
/// axis's `AwaitingGeneratorSupport` treatment.
#[test]
fn generator_gap_wordings_await_alef_support() {
    let body = "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'\n";
    assert_eq!(verdicts_for(body, "go"), vec![SkipVerdict::AwaitingGeneratorSupport]);
}

/// ~keep The wording every streaming backend now emits when its renderer cannot express an
/// assertion type. It replaces `// streaming field '<f>': assertion type '<t>' not rendered`,
/// which matched no registered shape and carried no `skipped:` prefix, so it was invisible to
/// this gate *and* to a grep census. Asserting the verdict rather than the text is the point:
/// a test that only grepped for the new text would pass on an unregistered wording too.
#[test]
fn the_streaming_assertion_type_wording_awaits_alef_support() {
    let line = super::assertion_type_skip::streaming_assertion_type_skip_line("    ", "//", "chunks", "matches_regex");
    let body = format!("{line}\n");
    assert_eq!(verdicts_for(&body, "go"), vec![SkipVerdict::AwaitingGeneratorSupport]);
}

/// The value-narrowing sibling: a streaming assertion type alef implements, whose fixture value
/// does not survive the renderer's `as_u64()` / string narrowing. Also alef's debt.
#[test]
fn the_streaming_assertion_value_wording_awaits_alef_support() {
    let line = super::assertion_type_skip::streaming_assertion_value_skip_line("    ", "//", "chunks", "count_min");
    let body = format!("{line}\n");
    assert_eq!(verdicts_for(&body, "dart"), vec![SkipVerdict::AwaitingGeneratorSupport]);
}

/// LanguageLimitation-classified wordings are counted as a real limitation, not alef's debt.
#[test]
fn language_limitation_wordings_are_counted_as_limitations() {
    let body = "        // skipped: field 'content' is a scalar String without meaningful .count\n";
    assert_eq!(verdicts_for(body, "swift"), vec![SkipVerdict::Limitation]);
}

/// Regression control: an ordinary field-availability skip must not be picked up by this gate
/// — the two funnels (`FieldSkip` / `AssertionTypeSkip`) stay disjoint.
#[test]
fn field_availability_markers_are_not_recorded_by_this_gate() {
    let body = "    // skipped: field 'chunks' not available on result type\n";
    assert!(verdicts_for(body, "python").is_empty());
}

#[test]
fn a_body_with_no_marker_records_nothing() {
    assert!(verdicts_for("    assert result.count == 1\n", "python").is_empty());
}

/// The one-line summary must call out how many of the skips came from the assertion-type axis
/// rather than the field axis, or the two are indistinguishable in the number a consumer reads.
#[test]
fn summary_calls_out_assertion_type_skips_separately() {
    let _ = take_skip_records();
    fail_on_unsupported_assertion_type_markers(
        "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'\n",
        "go",
        "traversal_smoke",
    );
    let summary = skip_summary(&take_skip_records()).expect("summary");
    assert!(
        summary.contains("1 from an assertion type this backend cannot render"),
        "got: {summary}"
    );
}
