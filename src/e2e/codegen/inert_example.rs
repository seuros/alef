//! The one place that decides whether a fixture's rendered example still asserts anything.
//!
//! ~keep [`super::field_skip`] and [`super::assertion_type_skip`] made a *dropped* assertion
//! visible: the backend renders a `skipped: ...` comment instead of the check, and both funnels
//! drain through the shared ledger so the drop is counted. Neither funnel changes what is
//! published. An example whose assertions ALL funnelled into markers is therefore still emitted
//! as an ordinary test — a body that runs the call, collects the result and then asserts nothing,
//! which passes whatever the code under test does. So is an example whose assertions rendered
//! nothing at all. The two shapes are indistinguishable at runtime: both are green, permanently.
//!
//! The emptiness is decidable in one place because every backend already hands its fully-rendered
//! assertion body to [`super::fail_on_unavailable_field_markers`] at exactly the right moment —
//! after the last assertion is rendered and before the body is spliced into the emitted test. Two
//! questions have to be answered there, and only one of them the ledger can answer alone:
//!
//! - *how many markers did this example produce, and whose debt are they?* — the ledger knows.
//!   [`super::peek_skip_records`] reads back the records the funnels just pushed for this
//!   (language, fixture) pair, including each one's [`super::SkipVerdict`], so the caller can tell
//!   a consumer-fixable unresolved path from alef's own generator debt without re-matching any
//!   wording.
//! - *did anything else render?* — the ledger cannot know. It only ever sees the lines that carry
//!   a registered marker; a body of three markers and a body of three markers plus a real
//!   assertion push identical records. That half has to come from the rendered text, which is why
//!   this module takes the body as well as reading the ledger.
//!
//! Hence one check, at one call site shape, rather than sixteen independent ones — and
//! [`has_executable_line`] is the single copy of a predicate four backends had already
//! hand-rolled separately (python, typescript, elixir, r), each with its own comment character.

use crate::e2e::fixture::Assertion;

use super::{SkipVerdict, peek_skip_records};

/// ~keep The union of every comment opener the generated e2e languages use, deliberately kept
/// tiny: this set is only ever consulted to decide that a line is NOT executable, so a missing
/// opener merely makes an example look live (safe — nothing is withheld) while a wrong entry
/// could make a real assertion look like a comment and withhold working coverage. `--`, `*`, `;`
/// and `%` are therefore excluded even though some languages comment with them: `--x;` and
/// `*out = ...` are C statements, and no language alef emits e2e code for comments that way.
const COMMENT_OPENERS: &[&str] = &["//", "#", "/*"];

/// True when `body` carries at least one line that can execute — neither blank nor a comment.
///
/// A rendered skip marker is a comment by construction (the funnels render
/// `<indent><comment-open> skipped: ...`), so a body made only of markers answers `false`.
pub(crate) fn has_executable_line(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !COMMENT_OPENERS.iter().any(|opener| trimmed.starts_with(opener))
    })
}

/// Why an example ended up with nothing to assert — and therefore who can fix it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InertCause {
    /// ~keep At least one marker on this body is an unacknowledged authoring gap: a field path
    /// the availability oracle could not resolve. A fixture or config edit makes the assertion
    /// run, so the honest emission is one that FAILS and names the gap. Under the default strict
    /// setting the run has already been failed by [`super::strict_assertion_failure`]; this cause
    /// exists for the disarmed run, which must still not produce a green test.
    UnresolvedFieldPath,
    /// ~keep Every marker is alef's own generator debt or a real language/ABI limit (or the
    /// fixture acknowledged it). No consumer edit clears it, so a failing test would only force a
    /// blanket opt-out and rebuild the silent skip with extra ceremony — the argument
    /// `field_skip::SkipClass::GeneratorGap` already makes. The example must still not be
    /// published as passing.
    AwaitedOrLimited,
    /// The fixture declared assertions and the backend rendered neither a check nor a marker.
    /// The oldest shape of this defect and the only one no funnel can see. ~keep
    RenderedNothing,
}

/// One example that would have been published with nothing to assert.
#[derive(Debug, Clone)]
pub(crate) struct InertExample {
    pub(crate) language: String,
    pub(crate) fixture_id: String,
    /// How many registered skip markers the funnels recorded for this example.
    pub(crate) markers: usize,
    pub(crate) cause: InertCause,
}

impl InertExample {
    /// The sentence a generated refusal carries, so the reason survives into the emitted file
    /// rather than living only in alef's run log.
    pub(crate) fn reason(&self) -> String {
        match self.cause {
            InertCause::UnresolvedFieldPath => format!(
                "alef resolved no assertion for fixture `{}`: {} assertion(s) name a field path \
                 the availability oracle rejected",
                self.fixture_id, self.markers,
            ),
            InertCause::AwaitedOrLimited => format!(
                "alef rendered no runnable expectation for fixture `{}`: all {} declared \
                 assertion(s) were skipped (see the markers above)",
                self.fixture_id, self.markers,
            ),
            InertCause::RenderedNothing => format!(
                "alef rendered no runnable expectation for fixture `{}`: its declared assertions \
                 produced no check and no skip marker",
                self.fixture_id,
            ),
        }
    }
}

thread_local! {
    /// ~keep Thread-local for the same reason the skip ledger in `super` is: generation runs the
    /// backends on the driver's thread, and a thread-local keeps `#[test]` cases independent for
    /// free.
    static INERT_LEDGER: std::cell::RefCell<Vec<InertExample>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Drain every inert example recorded on this thread since the last drain.
pub(crate) fn take_inert_examples() -> Vec<InertExample> {
    INERT_LEDGER.with(|ledger| std::mem::take(&mut *ledger.borrow_mut()))
}

/// The one-line summary of examples that could not be published as passing tests, or `None`.
pub(crate) fn inert_summary(records: &[InertExample]) -> Option<String> {
    if records.is_empty() {
        return None;
    }
    let count = |cause: InertCause| records.iter().filter(|record| record.cause == cause).count();
    let languages: std::collections::BTreeSet<&str> = records.iter().map(|record| record.language.as_str()).collect();
    Some(format!(
        "{} generated example(s) across {} language(s) had no runnable expectation and were \
         refused rather than published as passing tests: {} with an unresolved field path, {} \
         awaiting alef support or blocked by a language limit, {} that rendered nothing at all",
        records.len(),
        languages.len(),
        count(InertCause::UnresolvedFieldPath),
        count(InertCause::AwaitedOrLimited),
        count(InertCause::RenderedNothing),
    ))
}

/// Record a refusal on the run's ledger.
///
/// ~keep Separate from [`inert_verdict`] because the verdict is also consulted by backends that go
/// on to publish a real (if weak) fallback expectation instead of refusing — ruby's
/// `expect(result).not_to be_nil` is one. Recording inside the verdict would count those as
/// refused examples and inflate the end-of-run number with coverage that is in fact running.
pub(crate) fn record_refusal(refusal: &InertExample) {
    INERT_LEDGER.with(|ledger| ledger.borrow_mut().push(refusal.clone()));
}

/// The inert verdict for one fixture's rendered assertion body, or `None` when the example still
/// asserts something.
///
/// Call this immediately after the backend's own [`super::fail_on_unavailable_field_markers`] /
/// [`super::fail_on_unsupported_assertion_type_markers`] call, on the same body: the ledger read
/// below only sees markers those funnels have already recorded.
///
/// ~keep A fixture that declares NO assertions is never refused. That is a deliberate, long
/// standing "just call it" smoke-test contract (see python's
/// `should_discard_result_when_force_bind_result_is_unset_and_unused`), and refusing it would
/// delete working coverage — the failure mode in the other direction, and the one that is silent.
pub(crate) fn inert_verdict(
    assertions_body: &str,
    language: &str,
    fixture_id: &str,
    assertions: &[Assertion],
) -> Option<InertExample> {
    if assertions.is_empty() || has_executable_line(assertions_body) {
        return None;
    }
    let records = peek_skip_records(language, fixture_id);
    let cause = if records
        .iter()
        .any(|record| record.verdict == SkipVerdict::UnacknowledgedGap)
    {
        InertCause::UnresolvedFieldPath
    } else if records.is_empty() {
        InertCause::RenderedNothing
    } else {
        InertCause::AwaitedOrLimited
    };
    Some(InertExample {
        language: language.to_string(),
        fixture_id: fixture_id.to_string(),
        markers: records.len(),
        cause,
    })
}

#[cfg(test)]
mod tests {
    use super::{InertCause, has_executable_line, inert_summary, inert_verdict, record_refusal, take_inert_examples};
    use crate::e2e::codegen::field_skip::FieldSkip;
    use crate::e2e::fixture::Assertion;

    fn assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!("x")),
            ..Default::default()
        }
    }

    /// The control that must come first: a body carrying a real check is never refused, whatever
    /// else surrounds it. A refusal that is too broad deletes working coverage silently, which is
    /// the same defect pointing the other way. ~keep
    #[test]
    fn a_body_with_one_real_expectation_is_not_refused() {
        let _ = take_inert_examples();
        let body = format!(
            "    # skipped: {}\n    expect(result.title).to eq('x')\n",
            FieldSkip::NotAvailableOnResultType.message("metadata.title")
        );
        assert!(has_executable_line(&body), "control: the rendered check must be seen");
        assert!(
            inert_verdict(&body, "ruby", "control_fixture", &[assertion("metadata.title")]).is_none(),
            "an example with a real expectation must be published unchanged"
        );
        assert!(
            take_inert_examples().is_empty(),
            "nothing may be recorded for a live example"
        );
    }

    #[test]
    fn a_body_of_only_skip_markers_is_refused() {
        let _ = take_inert_examples();
        let body = format!(
            "    # skipped: {}\n    # skipped: {}\n",
            FieldSkip::StreamingAssertionOnUnsupportedField.message("stream.has_page_event"),
            FieldSkip::StreamingAssertionOnUnsupportedField.message("stream_complete"),
        );
        assert!(!has_executable_line(&body), "a comment-only body executes nothing");
        let refusal = inert_verdict(&body, "ruby", "stream_fixture", &[assertion("stream.has_page_event")])
            .expect("an all-markers example must be refused");
        assert_eq!(refusal.fixture_id, "stream_fixture");
        record_refusal(&refusal);
        assert_eq!(take_inert_examples().len(), 1, "the refusal must be recorded once");
    }

    #[test]
    fn an_empty_body_with_declared_assertions_is_refused_as_rendered_nothing() {
        let _ = take_inert_examples();
        let refusal =
            inert_verdict("", "ruby", "silent_fixture", &[assertion("metadata.title")]).expect("must be refused");
        assert_eq!(refusal.cause, InertCause::RenderedNothing);
        assert_eq!(refusal.markers, 0);
        let _ = take_inert_examples();
    }

    /// The deliberate "just call it" smoke test: no declared assertions, so there is nothing to
    /// have been dropped and nothing to refuse. ~keep
    #[test]
    fn a_fixture_that_declares_no_assertions_is_never_refused() {
        let _ = take_inert_examples();
        assert!(inert_verdict("", "ruby", "smoke_fixture", &[]).is_none());
        assert!(take_inert_examples().is_empty());
    }

    #[test]
    fn comment_openers_do_not_swallow_real_statements() {
        assert!(has_executable_line("    expect(x).to eq(1)\n"));
        assert!(has_executable_line("    assert result is not None\n"));
        assert!(has_executable_line("\t\tAssert.Equal(1, x);\n"));
        assert!(!has_executable_line("    // skipped: nothing here\n\n"));
        assert!(!has_executable_line("  /* skipped: nothing here */\n"));
    }

    #[test]
    fn the_summary_counts_each_cause_separately() {
        let _ = take_inert_examples();
        record_refusal(&inert_verdict("", "ruby", "one", &[assertion("a")]).expect("refused"));
        record_refusal(&inert_verdict("", "python", "two", &[assertion("b")]).expect("refused"));
        let records = take_inert_examples();
        let summary = inert_summary(&records).expect("two refusals must summarise");
        assert!(summary.starts_with("2 generated example(s)"), "got: {summary}");
        assert!(inert_summary(&[]).is_none());
    }
}
