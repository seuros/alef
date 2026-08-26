//! Per-invocation suppression of e2e diagnostics that have already been reported.
//!
//! `bin_cli::helpers::collect_managed_surface` renders the e2e stage twice for every crate --
//! once in local dep mode, once with `dep_mode` flipped to `Registry` -- because the two write to
//! different roots (`e2e.output` versus `e2e.registry.output`) and `alef verify` needs the file
//! surface of both. `alef all` does the same across its two substages. None of the config-vs-IR
//! validators reads `dep_mode`, so the second render recomputes a bit-identical diagnostic set;
//! logging it again produced two copies of every warning per crate. In a log that reads exactly
//! like the generator re-running per target, which is what one consumer reported it as -- but it
//! scales with crates, not targets.
//!
//! The log is a value a caller creates and hands to the passes it wants deduplicated, not a
//! process-wide static: suppression must end when the command that opened it ends. A static would
//! silence a genuinely new occurrence of the same diagnostic in any process that generates more
//! than once -- a long-running library consumer, or the whole test binary. ~keep

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use super::validate::ValidationError;

/// The `(file, message)` pairs one invocation has already reported.
///
/// Cloning shares the record; the handle is `Send + Sync` because
/// [`crate::bin_cli`]'s managed-surface stages run under rayon and the local and registry e2e
/// renders can land on different threads.
#[derive(Clone, Debug, Default)]
pub struct DiagnosticLog {
    reported: Arc<Mutex<HashSet<(String, String)>>>,
}

impl DiagnosticLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `(file, message)` and reports whether this invocation had not seen it before.
    ///
    /// Recovers from a poisoned lock rather than propagating the panic: this is suppression
    /// bookkeeping, and a validator that panicked on another thread must not also turn every
    /// later diagnostic into a second panic that hides the first. ~keep
    pub fn is_first_occurrence(&self, file: &str, message: &str) -> bool {
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        reported.insert((file.to_owned(), message.to_owned()))
    }
}

/// The subset of `diagnostics` this invocation has not reported yet, marked as reported.
///
/// Returns the diagnostics rather than logging them so each call site keeps its own severity
/// policy -- `crate::e2e::generate_e2e`'s loops split `Error` to `error!` and `Warning` to
/// `warn!`, while the `enforce_*` validators log every diagnostic at `warn!`. Only the
/// repetition is removed here; which diagnostics exist, and which of them abort generation, are
/// unchanged. ~keep
pub fn unreported<'a>(diagnostics: &'a [ValidationError], log: &DiagnosticLog) -> Vec<&'a ValidationError> {
    diagnostics
        .iter()
        .filter(|diagnostic| log.is_first_occurrence(&diagnostic.file, &diagnostic.message))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::validate::Severity;

    fn diagnostic(file: &str, message: &str) -> ValidationError {
        ValidationError {
            file: file.to_owned(),
            message: message.to_owned(),
            severity: Severity::Warning,
        }
    }

    #[test]
    fn a_second_identical_pass_reports_nothing_new() {
        let diagnostics = vec![
            diagnostic("calls.toml", "module 'x' is not importable"),
            diagnostic("fixtures/a.json", "unknown arg 'y'"),
        ];
        let log = DiagnosticLog::new();

        let first = unreported(&diagnostics, &log);
        let second = unreported(&diagnostics, &log);

        assert_eq!(first.len(), 2);
        assert_eq!(
            second.len(),
            0,
            "the registry pass must not repeat the local pass's diagnostics"
        );
    }

    /// The distinct set is the invariant that matters: a dedup that also dropped a real
    /// diagnostic would still halve the count. ~keep
    #[test]
    fn deduplication_preserves_every_distinct_diagnostic() {
        let diagnostics = vec![
            diagnostic("calls.toml", "module 'x' is not importable"),
            diagnostic("fixtures/a.json", "unknown arg 'y'"),
        ];
        let log = DiagnosticLog::new();

        let mut reported: Vec<(String, String)> = unreported(&diagnostics, &log)
            .into_iter()
            .chain(unreported(&diagnostics, &log))
            .map(|diagnostic| (diagnostic.file.clone(), diagnostic.message.clone()))
            .collect();
        reported.sort();

        assert_eq!(
            reported,
            vec![
                ("calls.toml".to_owned(), "module 'x' is not importable".to_owned()),
                ("fixtures/a.json".to_owned(), "unknown arg 'y'".to_owned()),
            ]
        );
    }

    #[test]
    fn diagnostics_differing_only_in_file_are_both_reported() {
        let diagnostics = vec![
            diagnostic("crate-a/alef.toml", "module 'x' is not importable"),
            diagnostic("crate-b/alef.toml", "module 'x' is not importable"),
        ];
        let log = DiagnosticLog::new();

        assert_eq!(unreported(&diagnostics, &log).len(), 2);
    }

    /// Suppression is scoped to the log, so the next command's identical finding is reported
    /// again instead of being swallowed by the previous one. ~keep
    #[test]
    fn a_fresh_log_reports_a_previously_reported_diagnostic_again() {
        let diagnostics = vec![diagnostic("calls.toml", "module 'x' is not importable")];
        let first_invocation = DiagnosticLog::new();
        let _ = unreported(&diagnostics, &first_invocation);

        let second_invocation = DiagnosticLog::new();

        assert_eq!(unreported(&diagnostics, &second_invocation).len(), 1);
    }

    #[test]
    fn a_clone_shares_the_record_with_its_original() {
        let diagnostics = vec![diagnostic("calls.toml", "module 'x' is not importable")];
        let log = DiagnosticLog::new();
        let shared = log.clone();

        let _ = unreported(&diagnostics, &log);

        assert_eq!(
            unreported(&diagnostics, &shared).len(),
            0,
            "a clone must share suppression so parallel stages agree"
        );
    }
}
