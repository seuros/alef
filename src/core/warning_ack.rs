//! Runtime engine for warning acknowledgements (task #540).
//!
//! The consumer's audit reached one design for acknowledging a warning without hiding a whole
//! class of them: an acknowledgement is keyed by BOTH the warning's exact identity and the
//! exact source target it fired for, a STALE acknowledgement (one that matches nothing this
//! run) fails the run rather than passing silently, and every match is counted so the
//! suppressed set is visible rather than invisible. This module is that engine; the config
//! schema it consumes lives in [`crate::core::config::warning_ack`].
//!
//! # Wiring a producer in
//!
//! 1. Build one [`AcknowledgementLedger`] per bounded run scope (e.g. once per crate's snippet
//!    generation pass) from that scope's configured
//!    [`WarningAcknowledgement`](crate::core::config::warning_ack::WarningAcknowledgement)
//!    entries, naming the [`AcknowledgeableWarningCategory`] variant(s) that scope may accept.
//! 2. Immediately before emitting a warning, call [`AcknowledgementLedger::check`] with the
//!    warning's category, exact identity, and exact source target.
//!    - [`AckOutcome::Acknowledged`] means skip emitting the warning; the match is already
//!      recorded.
//!    - [`AckOutcome::NotAcknowledged`] means emit the warning as usual, and SHOULD include
//!      `would_acknowledge` in its message so a consumer can act on it without guessing the
//!      config shape.
//! 3. After the scope finishes, call [`AcknowledgementLedger::finish`] and propagate its error:
//!    a [`WarningAckError::Stale`] MUST fail the run. On success, report
//!    [`AcknowledgementReport::matched_count`] when nonzero -- the suppressed set must stay
//!    visible, never silent.
//!
//! `src/e2e/codegen/presentation.rs` (owned by a separate task fixing the underlying
//! tagged-union/method-crossing bug this warning reports on) is the intended next caller for
//! [`AcknowledgeableWarningCategory::VirtualFieldPath`]; this module does not touch that file.
//! Until it is wired in, any `virtual_field_path` entry a consumer configures will report as
//! stale on `finish` for whichever ledger scope accepts it, which is the structurally correct
//! answer -- nothing has matched it yet. ~keep

use crate::core::config::warning_ack::{AcknowledgeableWarningCategory, WarningAcknowledgement};

/// The result of checking one warning occurrence against a ledger's configured
/// acknowledgements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckOutcome {
    /// A configured entry matched this exact `(category, identity, target)`; the caller must
    /// not emit its warning. `matched_entry` is the config entry that matched, for a caller
    /// that wants to log the suppression itself.
    Acknowledged { matched_entry: String },
    /// No configured entry matched; the caller must emit its warning as usual.
    /// `would_acknowledge` is the exact `alef.toml` entry that would silence this occurrence,
    /// meant to be embedded in the warning message itself -- see this module's doc comment.
    NotAcknowledged { would_acknowledge: String },
}

/// What a finished ledger reports: the suppressed set stated plainly, never left invisible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AcknowledgementReport {
    /// Total number of warning occurrences a configured entry matched this run. Zero when no
    /// acknowledgements are configured or none fired.
    pub matched_count: usize,
    /// One rendered line per acknowledgement entry that matched at least once, each naming the
    /// entry and how many occurrences it matched.
    pub matched_entries: Vec<String>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum WarningAckError {
    /// Requirement 2: the single most important guarantee this engine exists to provide. An
    /// acknowledgement that matched nothing -- because the warning was fixed, never fired for
    /// that identity/target, or was mistyped -- must fail the run, not linger as dead
    /// configuration silently suppressing nothing.
    #[error(
        "{count} acknowledged warning(s) matched nothing this run and must be removed or \
         corrected (the warning may have been fixed, or never fired for that identity/target): \
         {entries}"
    )]
    Stale { count: usize, entries: String },
    /// A category exists on [`AcknowledgeableWarningCategory`] but is not one this particular
    /// ledger scope accepts -- e.g. a `virtual_field_path` entry configured somewhere only
    /// `doc_snippet_reserved_domain` is meaningful. Caught up front rather than reported as
    /// merely "stale" so the diagnostic names the real defect: wrong location, not a fixed
    /// warning.
    #[error(
        "`{category}` cannot be acknowledged here; this location only accepts: {allowed}. \
         Offending entry: identity = \"{identity}\", target = \"{target}\""
    )]
    OutOfScope {
        category: AcknowledgeableWarningCategory,
        identity: String,
        target: String,
        allowed: String,
    },
}

/// Accumulates acknowledgement matches over one bounded run scope and enforces task #540's two
/// hard guarantees at [`Self::finish`]: every match is counted, and every configured entry that
/// matched nothing fails the run.
#[derive(Debug)]
pub struct AcknowledgementLedger {
    scope: Vec<AcknowledgeableWarningCategory>,
    entries: Vec<WarningAcknowledgement>,
    matched_counts: Vec<usize>,
}

impl AcknowledgementLedger {
    /// Build a ledger that only accepts acknowledgements for the categories named in `scope`.
    ///
    /// Rejects up front (rather than deferring to [`Self::finish`]) when `entries` contains a
    /// category outside `scope`: that is a configuration-location mistake, not a stale
    /// acknowledgement, and deserves its own diagnostic.
    pub fn new(
        scope: &[AcknowledgeableWarningCategory],
        entries: Vec<WarningAcknowledgement>,
    ) -> Result<Self, WarningAckError> {
        if let Some(entry) = entries.iter().find(|entry| !scope.contains(&entry.category)) {
            return Err(WarningAckError::OutOfScope {
                category: entry.category,
                identity: entry.identity.clone(),
                target: entry.target.clone(),
                allowed: scope
                    .iter()
                    .map(|category| category.config_value())
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        let matched_counts = vec![0; entries.len()];
        Ok(Self {
            scope: scope.to_vec(),
            entries,
            matched_counts,
        })
    }

    /// Check one warning occurrence against the configured acknowledgements.
    ///
    /// Matching is exact-string equality on `(category, identity, target)` only -- no globs, no
    /// prefixes, no class-wide wildcard. `category` must be one this ledger's `scope` was built
    /// with; calling with an out-of-scope category is a caller bug (a category this ledger was
    /// never told to service cannot be meaningfully checked), so debug builds assert it rather
    /// than silently matching nothing.
    pub fn check(&mut self, category: AcknowledgeableWarningCategory, identity: &str, target: &str) -> AckOutcome {
        debug_assert!(
            self.scope.contains(&category),
            "checked warning category `{category}` outside this ledger's declared scope; add it \
             to the scope passed to AcknowledgementLedger::new"
        );
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.category == category && entry.identity == identity && entry.target == target)
        {
            self.matched_counts[index] += 1;
            let entry = &self.entries[index];
            return AckOutcome::Acknowledged {
                matched_entry: WarningAcknowledgement::config_entry_for(entry.category, &entry.identity, &entry.target),
            };
        }
        AckOutcome::NotAcknowledged {
            would_acknowledge: WarningAcknowledgement::config_entry_for(category, identity, target),
        }
    }

    /// Close the ledger: fail when any configured entry matched nothing, otherwise report the
    /// matched set.
    pub fn finish(self) -> Result<AcknowledgementReport, WarningAckError> {
        let stale: Vec<&WarningAcknowledgement> = self
            .entries
            .iter()
            .zip(&self.matched_counts)
            .filter_map(|(entry, count)| (*count == 0).then_some(entry))
            .collect();
        if !stale.is_empty() {
            let entries = stale
                .iter()
                .map(|entry| WarningAcknowledgement::config_entry_for(entry.category, &entry.identity, &entry.target))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(WarningAckError::Stale {
                count: stale.len(),
                entries,
            });
        }
        let matched_entries = self
            .entries
            .iter()
            .zip(&self.matched_counts)
            .filter(|(_, count)| **count > 0)
            .map(|(entry, count)| {
                format!(
                    "{} (matched {count}x)",
                    WarningAcknowledgement::config_entry_for(entry.category, &entry.identity, &entry.target)
                )
            })
            .collect();
        let matched_count = self.matched_counts.iter().sum();
        Ok(AcknowledgementReport {
            matched_count,
            matched_entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(category: AcknowledgeableWarningCategory, identity: &str, target: &str) -> WarningAcknowledgement {
        WarningAcknowledgement {
            category,
            identity: identity.to_string(),
            target: target.to_string(),
            reason: None,
        }
    }

    const RESERVED_DOMAIN: AcknowledgeableWarningCategory = AcknowledgeableWarningCategory::DocSnippetReservedDomain;
    const VIRTUAL_FIELD: AcknowledgeableWarningCategory = AcknowledgeableWarningCategory::VirtualFieldPath;

    /// The decisive test: an acknowledgement whose warning no longer fires must fail the run,
    /// not pass silently. This is the guarantee the entire mechanism exists to provide -- a
    /// suppression mechanism with no way to fail on staleness is the vacuous check task #540
    /// warns against.
    #[test]
    fn a_stale_acknowledgement_that_matched_nothing_fails_finish() {
        let ledger = AcknowledgementLedger::new(
            &[RESERVED_DOMAIN],
            vec![entry(RESERVED_DOMAIN, "extract_uri", "python")],
        )
        .expect("scope accepts the configured category");

        // No `check` call ever happens for this identity/target -- the warning it names never
        // fired this run.
        let error = ledger
            .finish()
            .expect_err("an acknowledgement that matched nothing must fail");

        match error {
            WarningAckError::Stale { count, entries } => {
                assert_eq!(count, 1, "exactly one entry matched nothing");
                assert!(
                    entries.contains("extract_uri") && entries.contains("python"),
                    "the failure must name the stale entry: {entries}"
                );
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn a_matched_acknowledgement_suppresses_and_finish_succeeds() {
        let mut ledger = AcknowledgementLedger::new(
            &[RESERVED_DOMAIN],
            vec![entry(RESERVED_DOMAIN, "extract_uri", "python")],
        )
        .expect("scope accepts the configured category");

        let outcome = ledger.check(RESERVED_DOMAIN, "extract_uri", "python");
        assert!(
            matches!(outcome, AckOutcome::Acknowledged { .. }),
            "an exact identity+target match must acknowledge: {outcome:?}"
        );

        let report = ledger.finish().expect("a fully-matched ledger must not fail");
        assert_eq!(
            report.matched_count, 1,
            "the matched count must be nonzero and accurate"
        );
        assert_eq!(report.matched_entries.len(), 1);
    }

    /// Requirement 1, target half: an acknowledgement for one source target must not silence
    /// the same identity firing for a different target.
    #[test]
    fn an_acknowledgement_for_a_different_target_does_not_apply() {
        let mut ledger = AcknowledgementLedger::new(
            &[RESERVED_DOMAIN],
            vec![entry(RESERVED_DOMAIN, "extract_uri", "python")],
        )
        .expect("scope accepts the configured category");

        let outcome = ledger.check(RESERVED_DOMAIN, "extract_uri", "go");
        match outcome {
            AckOutcome::NotAcknowledged { would_acknowledge } => {
                assert!(
                    would_acknowledge.contains("\"go\""),
                    "provenance must name the target actually observed, not the configured one: {would_acknowledge}"
                );
            }
            AckOutcome::Acknowledged { .. } => panic!("a target mismatch must never acknowledge"),
        }

        // The configured `python` entry matched nothing (only `go` was checked), so the ledger
        // must still fail overall -- a target mismatch is exactly the stale case.
        let error = ledger
            .finish()
            .expect_err("the configured entry never matched its target");
        assert!(matches!(error, WarningAckError::Stale { count: 1, .. }));
    }

    /// Requirement 1, identity half: the same target with a different identity must not match
    /// either.
    #[test]
    fn an_acknowledgement_for_a_different_identity_does_not_apply() {
        let mut ledger = AcknowledgementLedger::new(
            &[RESERVED_DOMAIN],
            vec![entry(RESERVED_DOMAIN, "extract_uri", "python")],
        )
        .expect("scope accepts the configured category");

        let outcome = ledger.check(RESERVED_DOMAIN, "other_fixture", "python");
        assert!(
            matches!(outcome, AckOutcome::NotAcknowledged { .. }),
            "an identity mismatch must never acknowledge: {outcome:?}"
        );
    }

    /// Requirement 3: the matched-count report must be nonzero and accurate, including when one
    /// entry matches more than once (e.g. the same fixture rendered for the same target across
    /// more than one call site in a run).
    #[test]
    fn matched_count_is_nonzero_and_accurate_across_repeated_matches() {
        let mut ledger = AcknowledgementLedger::new(
            &[RESERVED_DOMAIN],
            vec![
                entry(RESERVED_DOMAIN, "extract_uri", "python"),
                entry(RESERVED_DOMAIN, "batch_scrape", "go"),
            ],
        )
        .expect("scope accepts the configured categories");

        ledger.check(RESERVED_DOMAIN, "extract_uri", "python");
        ledger.check(RESERVED_DOMAIN, "extract_uri", "python");
        ledger.check(RESERVED_DOMAIN, "batch_scrape", "go");

        let report = ledger.finish().expect("every entry matched at least once");
        assert_eq!(report.matched_count, 3, "three occurrences matched across two entries");
        assert_eq!(report.matched_entries.len(), 2, "one report line per matched entry");
        assert!(report.matched_entries.iter().any(|line| line.contains("matched 2x")));
        assert!(report.matched_entries.iter().any(|line| line.contains("matched 1x")));
    }

    /// Requirement 4, structural half: a category this ledger's scope does not name is rejected
    /// even though the category itself is a legitimate, acknowledgeable variant elsewhere.
    #[test]
    fn a_category_outside_the_ledgers_scope_is_rejected_even_though_it_is_acknowledgeable_elsewhere() {
        let error =
            AcknowledgementLedger::new(&[RESERVED_DOMAIN], vec![entry(VIRTUAL_FIELD, "result.0::Ok", "python")])
                .expect_err("a category outside this ledger's scope must be rejected");

        match error {
            WarningAckError::OutOfScope { category, allowed, .. } => {
                assert_eq!(category, VIRTUAL_FIELD);
                assert_eq!(allowed, "doc_snippet_reserved_domain");
            }
            other => panic!("expected OutOfScope, got {other:?}"),
        }
    }

    #[test]
    fn a_ledger_with_no_configured_entries_reports_zero_matches_and_never_fails() {
        let ledger =
            AcknowledgementLedger::new(&[RESERVED_DOMAIN], vec![]).expect("an empty entry list is always valid");
        let report = ledger
            .finish()
            .expect("no configured entries means nothing can be stale");
        assert_eq!(report.matched_count, 0);
        assert!(report.matched_entries.is_empty());
    }
}
