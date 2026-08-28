use super::artifact_preflight::ArtifactPreflight;
use super::{
    BatchKey, FailureReporter, RunnerConfig, ValidationOutcome, batch_level, finalize_result, session_for, session_key,
    session_lock_for, session_preparation_error, session_preparation_result,
};
use crate::snippets::error::Result;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Snippet, SnippetStatus, ValidationLevel, ValidationResult};
use crate::snippets::validators::{BatchValidation, SnippetValidator, ValidatorRegistry};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Instant;

/// The shared, read-only halves of a batch pass, so a per-group function stays inside the
/// argument budget while the pass runs its groups concurrently. Every field is a shared reference,
/// which is what lets one context be borrowed by every rayon worker at once. ~keep
struct BatchContext<'a> {
    snippets: &'a [Snippet],
    registry: &'a ValidatorRegistry,
    config: &'a RunnerConfig,
    sessions: &'a HashMap<String, ValidationSession>,
    session_locks: &'a HashMap<String, Mutex<()>>,
    reporter: &'a FailureReporter,
}

struct GroupedSnippets {
    groups: BTreeMap<BatchKey, Vec<usize>>,
    results: Vec<Option<ValidationResult>>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; every argument is a distinct read-only half of the pass"
)]
pub(super) fn validate_batches(
    snippets: &[Snippet],
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &HashMap<String, ValidationSession>,
    session_errors: &HashMap<String, crate::snippets::session::SessionPreparationError>,
    session_locks: &HashMap<String, Mutex<()>>,
    reporter: &FailureReporter,
    preflight: &ArtifactPreflight,
) -> Vec<Option<ValidationResult>> {
    let GroupedSnippets { groups, mut results } = group_batchable_snippets(
        snippets,
        registry,
        config,
        sessions,
        session_errors,
        reporter,
        preflight,
    );
    let context = BatchContext {
        snippets,
        registry,
        config,
        sessions,
        session_locks,
        reporter,
    };
    for (index, validated) in dispatch_groups(&context, groups) {
        results[index] = Some(validated);
    }
    results
}

/// Partitions the run into batch groups, resolving the snippets that never reach a validator at
/// all (a failed session preparation) on the spot.
fn group_batchable_snippets(
    snippets: &[Snippet],
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &HashMap<String, ValidationSession>,
    session_errors: &HashMap<String, crate::snippets::session::SessionPreparationError>,
    reporter: &FailureReporter,
    preflight: &ArtifactPreflight,
) -> GroupedSnippets {
    let mut results = vec![None; snippets.len()];
    let mut groups = BTreeMap::<BatchKey, Vec<usize>>::new();
    for (index, snippet) in snippets.iter().enumerate() {
        if let Some(preparation_error) = session_preparation_error(snippet, config, session_errors) {
            let failure = session_preparation_result(snippet, config, &preparation_error);
            reporter.record(&failure);
            results[index] = Some(failure);
            continue;
        }
        let session = session_for(snippet, sessions);
        if let Some(level) = batch_level(snippet, registry, config, session, preflight) {
            let key = (
                snippet.language,
                session_key(snippet, sessions).map(str::to_string),
                level,
            );
            groups.entry(key).or_default().push(index);
        }
    }
    GroupedSnippets { groups, results }
}

/// Runs every batch group concurrently and returns the positioned results to merge.
///
/// Two groups with different session *names* usually run different toolchains against different
/// scratch trees, so nothing needs to serialize them but this dispatch — with sixteen languages
/// configured, running them one after another made the batch pass as long as the sum of its
/// slowest members. But a session name is only a config label: `alef.toml` can alias two names
/// (e.g. a language fallback and an explicit binding-package target) to the identical
/// `cwd`/manifest, which resolves to the same physical workspace directory and the same session
/// fingerprint. Groups that share a session *name*, or whose sessions resolve to the same
/// fingerprint, still serialize -- on that fingerprint's mutex, via `session_lock_for` -- exactly
/// as they did before. ~keep
///
/// Each group returns its own `(index, result)` pairs rather than writing into a shared `Vec`, so
/// the position-indexed merge stays a single-threaded step the parallelism cannot race. ~keep
fn dispatch_groups(
    context: &BatchContext<'_>,
    groups: BTreeMap<BatchKey, Vec<usize>>,
) -> Vec<(usize, ValidationResult)> {
    // A rayon worker does not inherit the calling thread's current span (see `run_validation`'s
    // note on `pool.install`), so every group's Starting/Finished pair would lose its span context
    // the moment the groups stopped running on the caller's thread. ~keep
    let span = tracing::Span::current();
    groups
        .into_iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(key, indices)| span.in_scope(|| validate_group(context, &key, &indices)))
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect()
}

/// How much of a per-snippet budget a batch is expected to save.
///
/// A batch of N still compiles N snippets' worth of code, so its floor is one snippet's budget; what
/// it removes is N-1 toolchain startups, not the work. Charging it a flat `timeout_secs` -- the
/// per-invocation budget `validate_one` uses -- was correct only while a "batch" meant rust's handful
/// of snippets; with every language batching, one `dotnet build` or `tsc` now covers several hundred,
/// and the group would be killed as a toolchain timeout long before the compiler was finished. The
/// divisor keeps the grant well under N x the per-snippet budget, because avoiding N startups is
/// precisely why a batch is faster than the serial path it replaces. ~keep
const BATCH_TIMEOUT_SNIPPETS_PER_BUDGET: u64 = 8;

/// The wall-clock budget for one batched invocation covering `snippet_count` snippets.
fn batch_timeout_secs(per_snippet_secs: u64, snippet_count: usize) -> u64 {
    let count = u64::try_from(snippet_count).unwrap_or(u64::MAX);
    let grants = count.div_ceil(BATCH_TIMEOUT_SNIPPETS_PER_BUDGET).max(1);
    per_snippet_secs.saturating_mul(grants)
}

fn validate_group(context: &BatchContext<'_>, key: &BatchKey, indices: &[usize]) -> Vec<(usize, ValidationResult)> {
    let (language, session_target, level) = key;
    let validator = context.registry.get(*language).expect("batch group validator");
    let session = session_target.as_deref().and_then(|value| context.sessions.get(value));
    let batch_snippets = indices
        .iter()
        .map(|index| &context.snippets[*index])
        .collect::<Vec<_>>();
    let timeout_secs = batch_timeout_secs(context.config.timeout_secs, batch_snippets.len());
    tracing::info!(
        language = %language,
        snippet_count = batch_snippets.len(),
        timeout_secs,
        "Starting batched snippet validation"
    );
    let started = Instant::now();
    let batch = run_batch(context, validator, session, *level, &batch_snippets, timeout_secs);
    // `supports_batching` only screens out languages that never batch; a validator that
    // does support it (rust) can still decline a specific group — e.g. `Run` snippets, which
    // must execute one at a time — and return `None` here. Every `Starting` above must reach
    // a matching resolution, so this is logged explicitly instead of silently falling through
    // to the per-snippet path the caller reports on separately. ~keep
    let Some(batch) = batch else {
        tracing::info!(
            language = %language,
            snippet_count = batch_snippets.len(),
            "Batch validation declined for this group; falling back to per-snippet validation"
        );
        return Vec::new();
    };
    let BatchOutcome { values, timed_out } = batch_statuses(batch, indices.len());
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        language = %language,
        snippet_count = batch_snippets.len(),
        duration_ms,
        "Finished batched snippet validation"
    );
    finalize_group(
        context,
        validator,
        session,
        *level,
        &batch_snippets,
        values,
        indices,
        duration_ms,
        timed_out,
    )
}

/// Invokes the group's validator, holding the session's lock for the whole invocation when the
/// group has one.
///
/// Looked up by the session's *fingerprint* (via `session_lock_for`), not by its config name --
/// two differently-named sessions that alias the same physical workspace directory must share one
/// `Mutex`, or the exclusivity this lock exists to provide never actually holds between them. ~keep
fn run_batch(
    context: &BatchContext<'_>,
    validator: &dyn SnippetValidator,
    session: Option<&ValidationSession>,
    level: ValidationLevel,
    batch_snippets: &[&Snippet],
    timeout_secs: u64,
) -> Option<Result<BatchValidation>> {
    let validation = || validator.validate_batch_in_session(batch_snippets, level, timeout_secs, session);
    match session_lock_for(session, context.session_locks) {
        Some(lock) => lock.lock().ok().and_then(|_guard| validation()),
        None => validation(),
    }
}

/// One status per snippet in a group, plus whether the group's single invocation was killed at its
/// deadline rather than reporting on any of them.
struct BatchOutcome {
    values: BatchValidation,
    timed_out: bool,
}

/// Normalizes a batch validator's return into exactly one status per snippet, so a validator that
/// miscounts or fails outright still resolves every snippet the group claimed.
///
/// A timed-out group is the sharpest form of the accounting defect this flag fixes: one expired
/// stopwatch is charged to every snippet in the group, so a single overrun batch could contribute
/// hundreds to a count a reader would take for hundreds of broken snippets. The status stays
/// `Error` -- the snippets really were not validated -- but `timed_out` lets the summary say which
/// kind of number it is. ~keep
fn batch_statuses(batch: Result<BatchValidation>, expected: usize) -> BatchOutcome {
    match batch {
        Ok(values) if values.len() == expected => BatchOutcome {
            values,
            timed_out: false,
        },
        Ok(values) => {
            let message = format!(
                "batch validator returned {} results for {expected} snippets",
                values.len()
            );
            BatchOutcome {
                values: vec![(SnippetStatus::Error, Some(message)); expected],
                timed_out: false,
            }
        }
        Err(error) => {
            let timed_out = matches!(error, crate::snippets::error::Error::Timeout { .. });
            BatchOutcome {
                values: vec![(SnippetStatus::Error, Some(error.to_string())); expected],
                timed_out,
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; splitting it further would only move the arguments"
)]
fn finalize_group(
    context: &BatchContext<'_>,
    validator: &dyn SnippetValidator,
    session: Option<&ValidationSession>,
    level: ValidationLevel,
    batch_snippets: &[&Snippet],
    values: BatchValidation,
    indices: &[usize],
    duration_ms: u64,
    timed_out: bool,
) -> Vec<(usize, ValidationResult)> {
    let mut finalized = Vec::with_capacity(indices.len());
    for ((index, snippet), (status, message)) in indices.iter().copied().zip(batch_snippets).zip(values) {
        let value = finalize_result(
            snippet,
            validator,
            context.config,
            session,
            level,
            ValidationOutcome {
                status,
                message,
                duration_ms,
                timed_out,
            },
        );
        context.reporter.record(&value);
        finalized.push((index, value));
    }
    finalized
}

#[cfg(test)]
mod tests {
    use super::batch_timeout_secs;

    /// A batch of several hundred snippets is one invocation of one compiler covering all of them.
    /// Charging it the per-snippet budget killed it as a toolchain timeout long before the compiler
    /// could finish, which is a failure mode batching itself introduced. ~keep
    #[test]
    fn a_batch_budget_grows_with_the_number_of_snippets_it_covers() {
        assert_eq!(batch_timeout_secs(120, 1), 120, "a single snippet gets one budget");
        assert_eq!(
            batch_timeout_secs(120, 8),
            120,
            "a batch under the divisor still gets one"
        );
        assert_eq!(
            batch_timeout_secs(120, 9),
            240,
            "one snippet past the divisor buys the next grant"
        );
        assert_eq!(
            batch_timeout_secs(120, 283),
            4320,
            "a full language's batch gets 36 grants"
        );
    }

    /// An empty group cannot be charged zero: a zero budget is an immediate timeout, and the
    /// grouping step can hand this function a group it later declines.
    #[test]
    fn an_empty_batch_still_gets_one_whole_budget() {
        assert_eq!(batch_timeout_secs(120, 0), 120);
    }

    /// The grant must stay well under N x the per-snippet budget: a batch's whole point is that it
    /// pays one toolchain startup instead of N, so granting it the serial path's full budget would
    /// let a genuinely hung compiler run for hours before the timeout fired. ~keep
    #[test]
    fn a_batch_budget_stays_far_below_the_serial_path_it_replaces() {
        let serial = 120 * 283;
        let granted = batch_timeout_secs(120, 283);
        assert!(
            granted <= serial / 4,
            "a 283-snippet batch was granted {granted}s against the {serial}s the serial path would spend; \
             the divisor is what keeps a hung compiler from running for hours before the timeout fires"
        );
    }

    use crate::snippets::runner::{RunnerConfig, run_validation};
    use crate::snippets::types::{
        Language, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel, ValidationResult,
    };
    use crate::snippets::validators::{SnippetValidator, ValidatorRegistry};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// How long a probing validator waits for a sibling group to join it before giving up and
    /// letting the assertion report the observed peak. Long enough to absorb thread-pool startup
    /// on a loaded machine, short enough that a regression fails the test instead of hanging it.
    const CONCURRENCY_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

    #[derive(Default)]
    struct ConcurrencyProbe {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        /// The longest any single validator actually waited for a sibling to join it, in
        /// milliseconds. `peak` alone can't distinguish "reached 2 quickly" from "gave up at the
        /// full `CONCURRENCY_PROBE_TIMEOUT` and never saw a sibling" -- this is what lets the
        /// final assertion report a real elapsed/bound/margin instead of just a bare count. ~keep
        max_waited_millis: AtomicU64,
    }

    struct ProbingBatchValidator {
        language: Language,
        probe: Arc<ConcurrencyProbe>,
    }

    impl SnippetValidator for ProbingBatchValidator {
        fn language(&self) -> Language {
            self.language
        }

        fn is_available(&self) -> bool {
            true
        }

        fn validate(
            &self,
            _snippet: &Snippet,
            _level: ValidationLevel,
            _timeout_secs: u64,
        ) -> crate::snippets::error::Result<(SnippetStatus, Option<String>)> {
            Ok((SnippetStatus::Pass, None))
        }

        fn validate_batch_in_session(
            &self,
            snippets: &[&Snippet],
            _level: ValidationLevel,
            _timeout_secs: u64,
            _session: Option<&crate::snippets::session::ValidationSession>,
        ) -> Option<crate::snippets::error::Result<Vec<(SnippetStatus, Option<String>)>>> {
            let entered = self.probe.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.probe.peak.fetch_max(entered, Ordering::SeqCst);
            let started = Instant::now();
            let deadline = started + CONCURRENCY_PROBE_TIMEOUT;
            while self.probe.peak.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            let waited_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.probe.max_waited_millis.fetch_max(waited_millis, Ordering::SeqCst);
            self.probe.in_flight.fetch_sub(1, Ordering::SeqCst);
            let language = self.language;
            Some(Ok(snippets
                .iter()
                .map(|_| (SnippetStatus::Fail, Some(format!("{language} batch"))))
                .collect()))
        }

        fn supports_batching(&self) -> bool {
            true
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }

        fn is_dependency_error(&self, _output: &str) -> bool {
            false
        }
    }

    fn snippet(language: Language) -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language,
            title: None,
            code: "example".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    fn probing_registry(probe: &Arc<ConcurrencyProbe>) -> ValidatorRegistry {
        let mut registry = ValidatorRegistry::new();
        for language in [Language::Rust, Language::Java] {
            registry.register(Box::new(ProbingBatchValidator {
                language,
                probe: Arc::clone(probe),
            }));
        }
        registry
    }

    fn batch_config() -> RunnerConfig {
        RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 4,
            cache_dir: None,
            ..RunnerConfig::default()
        }
    }

    /// Two groups keyed on different languages share no session lock, no toolchain and no scratch
    /// tree, so nothing but the dispatch itself can serialize them. Each validator refuses to
    /// return until it has seen a second group in flight, so a sequential dispatch cannot record a
    /// peak above one. ~keep
    #[test]
    fn batch_groups_with_different_keys_run_concurrently() {
        let probe = Arc::new(ConcurrencyProbe::default());
        let registry = probing_registry(&probe);
        let snippets = [snippet(Language::Rust), snippet(Language::Java)];

        let summary = run_validation(&snippets, &registry, &batch_config()).expect("validation completes");

        assert_eq!(summary.results.len(), 2);
        // A miss here means some validator gave up at the full `CONCURRENCY_PROBE_TIMEOUT`
        // without ever seeing a sibling join it -- report how close that wait came to its bound
        // before the bare peak count below, so a future failure is self-diagnosing. ~keep
        let waited = Duration::from_millis(probe.max_waited_millis.load(Ordering::SeqCst));
        crate::test_support::assert_elapsed_under(
            "the slowest validator waited for a sibling group to join it",
            waited,
            CONCURRENCY_PROBE_TIMEOUT,
        );
        assert_eq!(probe.peak.load(Ordering::SeqCst), 2);
    }

    /// Concurrency must not disturb result placement: every result has to land back on the snippet
    /// position it was produced for, whichever group finished first. ~keep
    #[test]
    fn concurrent_groups_keep_results_at_their_snippet_positions() {
        let probe = Arc::new(ConcurrencyProbe::default());
        let registry = probing_registry(&probe);
        let snippets = [
            snippet(Language::Rust),
            snippet(Language::Java),
            snippet(Language::Java),
            snippet(Language::Rust),
        ];

        let summary = run_validation(&snippets, &registry, &batch_config()).expect("validation completes");

        let messages = summary
            .results
            .iter()
            .map(|result: &ValidationResult| result.message.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![
                "rust batch".to_string(),
                "java batch".to_string(),
                "java batch".to_string(),
                "rust batch".to_string(),
            ]
        );
        assert_eq!(summary.failed, 4);
    }
}
