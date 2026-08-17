use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::session::{SessionSpec, prepare_sessions_isolated};
use crate::snippets::types::{
    DowngradeReason, RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel,
    ValidationResult,
};
use crate::snippets::validators::ValidatorRegistry;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Instant;

pub struct RunnerConfig {
    pub level: ValidationLevel,
    pub parallelism: usize,
    pub timeout_secs: u64,
    pub fail_fast: bool,
    pub deny_unclassified: bool,
    pub allowed_side_effects: Vec<SideEffectClass>,
    pub cache_dir: Option<std::path::PathBuf>,
    pub changed_only: bool,
    pub sessions: HashMap<String, SessionSpec>,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            level: ValidationLevel::Syntax,
            parallelism: available_parallelism(),
            timeout_secs: 120,
            fail_fast: false,
            deny_unclassified: false,
            allowed_side_effects: Vec::new(),
            cache_dir: Some(std::path::PathBuf::from(".alef/snippets")),
            changed_only: false,
            sessions: HashMap::new(),
        }
    }
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get)
}

/// Run validation over the provided snippets.
///
/// # Errors
///
/// Returns an error when the validation thread pool cannot be created.
pub fn run_validation(snippets: &[Snippet], registry: &ValidatorRegistry, config: &RunnerConfig) -> Result<RunSummary> {
    let preparation = prepare_sessions_isolated(&config.sessions, config.timeout_secs);
    let sessions = preparation.sessions;
    let session_errors = preparation.errors;
    let session_locks = sessions
        .keys()
        .map(|target| (target.clone(), Mutex::new(())))
        .collect::<HashMap<_, _>>();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.parallelism)
        .build()
        .map_err(|err| crate::snippets::error::Error::Other(format!("failed to build thread pool: {err}")))?;

    let fail_fast = config.fail_fast;
    // `rayon::ThreadPool::install` always runs its closure on a pool worker thread, never on the
    // calling thread itself, and `tracing::Span::enter` sets the "current span" through
    // thread-local state that a raw OS thread switch does not inherit. Every `tracing::info!` in
    // `fail_fast_results`/`parallel_results`/`validate_batches` therefore ran with no span
    // context at all unless the caller's span is captured and re-entered here explicitly. ~keep
    let calling_span = tracing::Span::current();
    let results: Vec<ValidationResult> = pool.install(|| {
        let _entered = calling_span.enter();
        if fail_fast {
            fail_fast_results(snippets, registry, config, &sessions, &session_errors, &session_locks)
        } else {
            parallel_results(snippets, registry, config, &sessions, &session_errors, &session_locks)
        }
    });

    Ok(RunSummary::from_results(results))
}

fn fail_fast_results(
    snippets: &[Snippet],
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &HashMap<String, crate::snippets::session::ValidationSession>,
    session_errors: &HashMap<String, String>,
    session_locks: &HashMap<String, Mutex<()>>,
) -> Vec<ValidationResult> {
    tracing::info!(
        snippet_count = snippets.len(),
        timeout_secs = config.timeout_secs,
        "Starting fail-fast snippet validation"
    );
    let started = Instant::now();
    let reporter = FailureReporter::new(snippets);
    let mut results = Vec::with_capacity(snippets.len());
    for snippet in snippets {
        let preparation_error = session_preparation_error(snippet, sessions, session_errors);
        let session = session_for(snippet, sessions);
        let lock = session_key(snippet, sessions).and_then(|key| session_locks.get(key));
        let result = validate_one(snippet, registry, config, session, lock, preparation_error);
        reporter.record(&result);
        let should_stop =
            preparation_error.is_none() && matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
        results.push(result);
        if should_stop {
            break;
        }
    }
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        snippet_count = results.len(),
        duration_ms,
        "Finished fail-fast snippet validation"
    );
    results
}

/// Dispatches every snippet a real batch validator didn't already claim through
/// `validate_batches` to `validate_one`. That fallback previously had no tracing of its own: for
/// every language without batching support (all but rust), a snippet's entire validation happened
/// here invisibly, with only the misleading `Starting batched...` log from `validate_batches`
/// (see `batch_level`) hinting anything ran at all. This wraps the fallback with its own
/// Starting/Finished pair so it is never silent. Logged per language, matching
/// `validate_batches`'s own granularity: a consumer correlating `Starting`/`Finished` pairs by
/// name needs every language that did work to name itself on both sides, not just the ones that
/// happened to go through a real batch. ~keep
fn parallel_results(
    snippets: &[Snippet],
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &HashMap<String, crate::snippets::session::ValidationSession>,
    session_errors: &HashMap<String, String>,
    session_locks: &HashMap<String, Mutex<()>>,
) -> Vec<ValidationResult> {
    let reporter = FailureReporter::new(snippets);
    let batched = validate_batches(
        snippets,
        registry,
        config,
        sessions,
        session_errors,
        session_locks,
        &reporter,
    );
    let fallback_counts = fallback_counts_by_language(snippets, &batched);
    for (language, count) in &fallback_counts {
        tracing::info!(
            language = %language,
            snippet_count = count,
            timeout_secs = config.timeout_secs,
            "Starting per-snippet validation"
        );
    }
    let started = Instant::now();
    let results = snippets
        .par_iter()
        .enumerate()
        .map(|(index, snippet)| {
            if let Some(result) = batched[index].clone() {
                return result;
            }
            let session = session_for(snippet, sessions);
            let lock = session_key(snippet, sessions).and_then(|key| session_locks.get(key));
            let result = validate_one(snippet, registry, config, session, lock, None);
            reporter.record(&result);
            result
        })
        .collect();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    for (language, count) in &fallback_counts {
        tracing::info!(
            language = %language,
            snippet_count = count,
            duration_ms,
            "Finished per-snippet validation"
        );
    }
    results
}

/// Per-language snippet counts among the entries `validate_batches` left unclaimed (`None`) —
/// the ones `parallel_results` dispatches to `validate_one`. `duration_ms` on the resulting
/// `Finished` events is the whole parallel fallback pass, not a per-language measurement (every
/// language's snippets run concurrently, not one language at a time), but the language name
/// itself is exact, which is what a `Starting`/`Finished` correlation by name needs. ~keep
fn fallback_counts_by_language(
    snippets: &[Snippet],
    batched: &[Option<ValidationResult>],
) -> BTreeMap<crate::snippets::types::Language, usize> {
    let mut counts = BTreeMap::new();
    for (snippet, entry) in snippets.iter().zip(batched) {
        if entry.is_none() {
            *counts.entry(snippet.language).or_insert(0_usize) += 1;
        }
    }
    counts
}

/// A language emits one `WARN` for its first failure, then one more every this many failures.
/// Sized so a pathological run (1,753 failures spread over six languages) produces on the order of
/// seventy lines rather than one per failure, while a language that fails a handful of times still
/// gets its first-failure line immediately.
const FAILURE_PROGRESS_STRIDE: usize = 25;

/// How much of a validator's own error output the first-failure event carries. Long enough for a
/// `javac`/`tsc` diagnostic's first lines (the part that names the actual problem), short enough
/// that a log line stays a log line.
const FAILURE_MESSAGE_PREVIEW_CHARS: usize = 400;

#[derive(Clone, Copy, Default)]
struct LanguageTally {
    completed: usize,
    failed: usize,
}

/// Emits snippet failures *while* a validation pass is running.
///
/// Everything reported here was already known at the moment each `ValidationResult` was built —
/// `validate_one`/`validate_batches` hold the status and the validator's message, and
/// `finalize_result` writes both to the result cache. None of it reached the log: a run that
/// produced 1,753 failures across six languages failing at 100% was indistinguishable from a
/// healthy run until the stage ended and the summary printed.
///
/// One event per failure would be as unreadable as silence, so the budget is bounded per language:
/// the first failure carries the validator's own message (the only part that says *what* broke),
/// subsequent failures are counted and surfaced every [`FAILURE_PROGRESS_STRIDE`], and each
/// language emits exactly one terminal event once its last snippet lands — which is mid-run for
/// every language but the slowest, because `parallel_results` interleaves all languages.
struct FailureReporter {
    totals: BTreeMap<crate::snippets::types::Language, usize>,
    tallies: Mutex<BTreeMap<crate::snippets::types::Language, LanguageTally>>,
    span: tracing::Span,
}

impl FailureReporter {
    fn new(snippets: &[Snippet]) -> Self {
        let mut totals = BTreeMap::new();
        for snippet in snippets {
            *totals.entry(snippet.language).or_insert(0_usize) += 1;
        }
        Self {
            totals,
            tallies: Mutex::new(BTreeMap::new()),
            // Recorded from the constructing thread, which `run_validation` has already put in the
            // caller's span, and re-entered on every emission. Most `record` calls happen on a
            // rayon worker that `pool.install`'s one-off `Span::enter` never reached, so without
            // this the events a consumer most needs to correlate would be the span-less ones. ~keep
            span: tracing::Span::current(),
        }
    }

    fn record(&self, result: &ValidationResult) {
        let language = result.snippet.language;
        let failed = matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
        // A poisoned tally lock costs reporting only. Unwrapping here would let a panic in some
        // other worker's reporting turn a reportable run into an aborted one, which is exactly the
        // failure mode this reporter exists to prevent. ~keep
        let Ok(mut tallies) = self.tallies.lock() else {
            return;
        };
        let tally = tallies.entry(language).or_default();
        tally.completed += 1;
        if failed {
            tally.failed += 1;
        }
        let tally = *tally;
        drop(tallies);

        let snippet_count = self.totals.get(&language).copied().unwrap_or(tally.completed);
        self.span.in_scope(|| {
            if failed && tally.failed == 1 {
                tracing::warn!(
                    language = %language,
                    path = %result.snippet.source_origin.path.display(),
                    line = result.snippet.source_origin.line,
                    snippet_count = snippet_count,
                    error = %failure_preview(result.message.as_deref()),
                    "First snippet validation failure for this language"
                );
            } else if failed && tally.failed % FAILURE_PROGRESS_STRIDE == 0 {
                tracing::warn!(
                    language = %language,
                    failed = tally.failed,
                    completed = tally.completed,
                    snippet_count = snippet_count,
                    "Snippet validation failures accumulating"
                );
            }
            if tally.completed < snippet_count {
                return;
            }
            if tally.failed > 0 {
                tracing::warn!(
                    language = %language,
                    failed = tally.failed,
                    snippet_count = snippet_count,
                    "Finished snippet validation for this language with failures"
                );
            } else {
                tracing::debug!(
                    language = %language,
                    snippet_count = snippet_count,
                    "Finished snippet validation for this language"
                );
            }
        });
    }
}

/// Flattens a validator's multi-line diagnostic onto one bounded log line. Truncation is by
/// character, not byte, so a diagnostic quoting non-ASCII source cannot panic on a split boundary.
fn failure_preview(message: Option<&str>) -> String {
    let joined = message
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if joined.is_empty() {
        return "<no validator output>".to_string();
    }
    match joined.char_indices().nth(FAILURE_MESSAGE_PREVIEW_CHARS) {
        Some((index, _)) => format!("{}...", &joined[..index]),
        None => joined,
    }
}

type BatchKey = (crate::snippets::types::Language, Option<String>, ValidationLevel);

struct ValidationOutcome {
    status: SnippetStatus,
    message: Option<String>,
    duration_ms: u64,
}

fn validate_batches(
    snippets: &[Snippet],
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &HashMap<String, crate::snippets::session::ValidationSession>,
    session_errors: &HashMap<String, String>,
    session_locks: &HashMap<String, Mutex<()>>,
    reporter: &FailureReporter,
) -> Vec<Option<ValidationResult>> {
    let mut results = vec![None; snippets.len()];
    let mut groups = BTreeMap::<BatchKey, Vec<usize>>::new();
    for (index, snippet) in snippets.iter().enumerate() {
        if let Some(message) = session_preparation_error(snippet, sessions, session_errors) {
            let failure = result(
                snippet,
                SnippetStatus::Error,
                config.level,
                config.level,
                Some(message.to_owned()),
                0,
            );
            reporter.record(&failure);
            results[index] = Some(failure);
            continue;
        }
        let session = session_for(snippet, sessions);
        if let Some(level) = batch_level(snippet, registry, config, session) {
            let key = (
                snippet.language,
                session_key(snippet, sessions).map(str::to_string),
                level,
            );
            groups.entry(key).or_default().push(index);
        }
    }

    for ((language, key, level), indices) in groups {
        let validator = registry.get(language).expect("batch group validator");
        let session = key.as_deref().and_then(|value| sessions.get(value));
        let batch_snippets = indices.iter().map(|index| &snippets[*index]).collect::<Vec<_>>();
        tracing::info!(
            language = %language,
            snippet_count = batch_snippets.len(),
            timeout_secs = config.timeout_secs,
            "Starting batched snippet validation"
        );
        let started = Instant::now();
        let validation = || validator.validate_batch_in_session(&batch_snippets, level, config.timeout_secs, session);
        let batch = match key.as_deref().and_then(|value| session_locks.get(value)) {
            Some(lock) => lock.lock().ok().and_then(|_guard| validation()),
            None => validation(),
        };
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
            continue;
        };
        let values = match batch {
            Ok(values) if values.len() == indices.len() => values,
            Ok(values) => {
                let message = format!(
                    "batch validator returned {} results for {} snippets",
                    values.len(),
                    indices.len()
                );
                vec![(SnippetStatus::Error, Some(message)); indices.len()]
            }
            Err(error) => vec![(SnippetStatus::Error, Some(error.to_string())); indices.len()],
        };
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::info!(
            language = %language,
            snippet_count = batch_snippets.len(),
            duration_ms,
            "Finished batched snippet validation"
        );
        for ((index, snippet), (status, message)) in indices.into_iter().zip(batch_snippets).zip(values) {
            let finalized = finalize_result(
                snippet,
                validator,
                config,
                session,
                level,
                ValidationOutcome {
                    status,
                    message,
                    duration_ms,
                },
            );
            reporter.record(&finalized);
            results[index] = Some(finalized);
        }
    }
    results
}

fn batch_level(
    snippet: &Snippet,
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    session: Option<&crate::snippets::session::ValidationSession>,
) -> Option<ValidationLevel> {
    if cached_result(snippet, config, session).is_some() || side_effect_rejection(snippet, config).is_some() {
        return None;
    }
    if let Some(annotation) = &snippet.annotation
        && annotation.kind == SnippetAnnotationKind::Skip
    {
        return None;
    }
    let validator = registry.get(snippet.language)?;
    // A validator that never overrides `validate_batch_in_session` always returns `None` from it,
    // so grouping its snippets here just logged a `Starting batched snippet validation` event
    // with no matching `Finished` — the group silently fell through to the per-snippet fallback
    // in `run_validation`, a codepath this function's caller never observed. Checking
    // `supports_batching` upfront skips the batch path (and its logging) entirely for a language
    // that was never going to use it. ~keep
    if !validator.supports_batching() {
        return None;
    }
    let level = capped_level(snippet, config, validator);
    validator.is_available_at(level).then_some(level)
}

/// The ceiling imposed by a `<!-- snippet:*-only -->` comment annotation, if any. Distinct from
/// `snippet.metadata.level` (a front-matter `level:` contract): an annotation is the author
/// suppressing validation below what the run requested, so `finalize_result` keeps it a
/// `Downgraded` cause, while `snippet.metadata.level` is read directly wherever the contract
/// counterpart is needed. ~keep
fn annotation_level_limit(snippet: &Snippet) -> Option<ValidationLevel> {
    snippet
        .annotation
        .as_ref()
        .and_then(|annotation| match annotation.kind {
            SnippetAnnotationKind::SyntaxOnly => Some(ValidationLevel::Syntax),
            SnippetAnnotationKind::CompileOnly => Some(ValidationLevel::Compile),
            SnippetAnnotationKind::TypeCheckOnly => Some(ValidationLevel::TypeCheck),
            SnippetAnnotationKind::Skip => None,
        })
}

/// The level implied by the snippet's own declarations, independent of the validator or
/// environment: an annotation lowers it as a downgrade; a front-matter `level:` lowers it as a
/// contract instead. Both narrow the level actually attempted the same way here — only
/// `finalize_result` tells the two apart, to decide whether hitting this level is a violation or
/// a satisfied request. ~keep
fn effective_validation_level(snippet: &Snippet, requested: ValidationLevel) -> ValidationLevel {
    [annotation_level_limit(snippet), snippet.metadata.level]
        .into_iter()
        .flatten()
        .fold(requested, ValidationLevel::min)
}

/// The level a validator will actually be invoked at: the requested level, narrowed by the
/// snippet's own declarations (`effective_validation_level`), by the validator's permanent
/// `max_level` ceiling, and by `achievable_level` — this run's environment-dependent limit (e.g.
/// no real type-checker binary on `PATH`). ~keep
fn capped_level(
    snippet: &Snippet,
    config: &RunnerConfig,
    validator: &dyn crate::snippets::validators::SnippetValidator,
) -> ValidationLevel {
    effective_validation_level(snippet, config.level)
        .min(validator.max_level())
        .min(validator.achievable_level(config.level))
}

/// Whether the validator can never reach `requested` for this snippet's language, in any
/// environment: either its permanent `max_level` sits below it, or its `achievable_level` gap is
/// declared structural (see `SnippetValidator::achievable_level_is_structural`). Both make a
/// strict request for `requested` unsatisfiable for this language regardless of the user's
/// environment, so `finalize_result` treats them the same way. ~keep
fn structurally_unreachable(
    validator: &dyn crate::snippets::validators::SnippetValidator,
    requested: ValidationLevel,
) -> bool {
    validator.max_level() < requested
        || (validator.achievable_level(requested) < requested && validator.achievable_level_is_structural(requested))
}

fn session_for<'a>(
    snippet: &Snippet,
    sessions: &'a HashMap<String, crate::snippets::session::ValidationSession>,
) -> Option<&'a crate::snippets::session::ValidationSession> {
    snippet
        .metadata
        .target
        .as_ref()
        .and_then(|target| sessions.get(&crate::snippets::types::Language::normalize_session_target(target)))
        .or_else(|| sessions.get(&snippet.language.to_string()))
}

fn session_key<'a>(
    snippet: &Snippet,
    sessions: &'a HashMap<String, crate::snippets::session::ValidationSession>,
) -> Option<&'a str> {
    let target = snippet
        .metadata
        .target
        .as_ref()
        .map(|target| crate::snippets::types::Language::normalize_session_target(target));
    if let Some(target) = target.as_deref()
        && sessions.contains_key(target)
    {
        return sessions.get_key_value(target).map(|(key, _)| key.as_str());
    }
    sessions
        .get_key_value(&snippet.language.to_string())
        .map(|(key, _)| key.as_str())
}

fn session_preparation_error<'a>(
    snippet: &Snippet,
    sessions: &HashMap<String, crate::snippets::session::ValidationSession>,
    errors: &'a HashMap<String, String>,
) -> Option<&'a str> {
    let target = snippet
        .metadata
        .target
        .as_ref()
        .map(|target| crate::snippets::types::Language::normalize_session_target(target));
    if let Some(target) = target.as_deref() {
        if let Some(error) = errors.get(target) {
            return Some(error);
        }
        if sessions.contains_key(target) {
            return None;
        }
    }
    errors.get(&snippet.language.to_string()).map(String::as_str)
}

fn validate_one(
    snippet: &Snippet,
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    session: Option<&crate::snippets::session::ValidationSession>,
    session_lock: Option<&Mutex<()>>,
    session_preparation_error: Option<&str>,
) -> ValidationResult {
    if let Some(message) = session_preparation_error {
        return result(
            snippet,
            SnippetStatus::Error,
            config.level,
            config.level,
            Some(message.to_owned()),
            0,
        );
    }
    if let Some(result) = cached_result(snippet, config, session) {
        return result;
    }

    if let Some(message) = side_effect_rejection(snippet, config) {
        return result(
            snippet,
            SnippetStatus::Skip,
            config.level,
            config.level,
            Some(message),
            0,
        );
    }

    if let Some(annotation) = &snippet.annotation
        && annotation.kind == SnippetAnnotationKind::Skip
    {
        return result(
            snippet,
            SnippetStatus::Skip,
            config.level,
            config.level,
            Some(skip_message("skipped via annotation", annotation.reason.as_deref())),
            0,
        );
    }

    let Some(validator) = registry.get(snippet.language) else {
        return result(
            snippet,
            SnippetStatus::Unavailable,
            config.level,
            config.level,
            Some(format!("no validator for {}", snippet.language)),
            0,
        );
    };

    let effective_level = capped_level(snippet, config, validator);
    if !validator.is_available_at(effective_level) {
        return result(
            snippet,
            SnippetStatus::Unavailable,
            config.level,
            config.level,
            Some(format!("{} toolchain not found", snippet.language)),
            0,
        );
    }

    let start = Instant::now();
    // `timeout_secs` is a per-invocation budget here, not a group budget. This path runs one
    // toolchain process per snippet (every validator except rust's non-`Run` batch), so sharing a
    // single wall-clock deadline across the language group let the first snippet consume it and
    // left the rest reported as toolchain timeouts for commands that were never spawned. Group
    // budgeting belongs to `validate_batches`, where one process really does cover N snippets. ~keep
    let validation = || validator.validate_in_session(snippet, effective_level, config.timeout_secs, session);
    let validation_result = match session_lock {
        Some(lock) => match lock.lock() {
            Ok(_guard) => validation(),
            Err(error) => Err(crate::snippets::error::Error::Other(format!(
                "locking {} snippet validation session: {error}",
                snippet.language
            ))),
        },
        None => validation(),
    };
    let (status, message) = match validation_result {
        Ok((status, message)) => (status, message),
        Err(err) => (SnippetStatus::Error, Some(err.to_string())),
    };
    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    finalize_result(
        snippet,
        validator,
        config,
        session,
        effective_level,
        ValidationOutcome {
            status,
            message,
            duration_ms,
        },
    )
}

struct ResultClassification {
    status: SnippetStatus,
    capability_capped: bool,
    downgrade_reason: Option<DowngradeReason>,
}

/// Decides, for a `Pass` outcome that landed below `config.level`, whether that gap is: fully
/// explained by the snippet's own front-matter `level:` contract (`Declared`, still `Pass`);
/// unsatisfiable for this language regardless of environment (`ValidatorCapability`,
/// `capability_capped` and still `Pass`); or a real degradation (`Downgraded`, caused by a
/// suppression `Annotation` or by the current `Environment`). A non-`Pass` outcome, or one that
/// already reached `config.level`, needs none of this and passes through unchanged. ~keep
fn classify_result(
    snippet: &Snippet,
    validator: &dyn crate::snippets::validators::SnippetValidator,
    config: &RunnerConfig,
    effective_level: ValidationLevel,
    status: SnippetStatus,
) -> ResultClassification {
    if status != SnippetStatus::Pass || effective_level >= config.level {
        return ResultClassification {
            status,
            capability_capped: false,
            downgrade_reason: None,
        };
    }

    let annotated_level = effective_validation_level(snippet, config.level);
    let structural = structurally_unreachable(validator, config.level);
    if annotated_level >= config.level && structural {
        return ResultClassification {
            status,
            capability_capped: true,
            downgrade_reason: Some(DowngradeReason::ValidatorCapability),
        };
    }

    let declared_binds = snippet
        .metadata
        .level
        .is_some_and(|level| config.level.min(level) == annotated_level);
    if effective_level == annotated_level && annotated_level < config.level && declared_binds {
        return ResultClassification {
            status,
            capability_capped: false,
            downgrade_reason: Some(DowngradeReason::Declared),
        };
    }

    let reason = if effective_level < annotated_level && structural {
        DowngradeReason::ValidatorCapability
    } else if effective_level < annotated_level {
        DowngradeReason::Environment
    } else {
        DowngradeReason::Annotation
    };
    ResultClassification {
        status: SnippetStatus::Downgraded,
        capability_capped: false,
        downgrade_reason: Some(reason),
    }
}

fn finalize_result(
    snippet: &Snippet,
    validator: &dyn crate::snippets::validators::SnippetValidator,
    config: &RunnerConfig,
    session: Option<&crate::snippets::session::ValidationSession>,
    effective_level: ValidationLevel,
    outcome: ValidationOutcome,
) -> ValidationResult {
    let ValidationOutcome {
        mut status,
        message,
        duration_ms,
    } = outcome;
    if status == SnippetStatus::Fail
        && effective_level == ValidationLevel::Syntax
        && let Some(error_output) = &message
        && validator.is_dependency_error(error_output)
    {
        status = SnippetStatus::Pass;
    }

    let classification = classify_result(snippet, validator, config, effective_level, status);
    let status = classification.status;
    let message = if classification.downgrade_reason == Some(DowngradeReason::Declared) {
        // Naming `config.level` here, not just `effective_level`, is load-bearing: a `Declared`
        // `Pass` is the one downgrade classification `print_summary` used to leave unreported
        // (see `reason_line` in `snippets::output`), so this is the only place an operator who
        // configured a stronger level than a snippet's front-matter `level:` allows learns both
        // halves of the gap — what they asked for and what actually ran. ~keep
        Some(format!(
            "requested {}, validated at declared level {effective_level}",
            config.level
        ))
    } else if status == SnippetStatus::Downgraded {
        Some(format!("requested {}, validated at {}", config.level, effective_level))
    } else if classification.capability_capped {
        Some(format!(
            "requested {}, validated at {} ({} validator caps at {})",
            config.level, effective_level, snippet.language, effective_level
        ))
    } else {
        message
    };
    // `downgrade_reason` is `Option` because most results (an ordinary `Pass`, any `Fail`,
    // `Skip`, `Error`, or `Unavailable`) have no reason in this taxonomy at all — that is a real
    // "not applicable", not a degraded default, so a required enum would need its own sentinel
    // variant and would not remove the risk of a construction site passing it by mistake. What
    // does remove that risk: `classify_result` is exhaustive over every case that produces
    // `Downgraded` or a `capability_capped` `Pass`, and is the only place that sets this field to
    // anything other than `None` — asserted here so a future change to `classify_result` that
    // silently drops the reason on one of those paths fails loudly instead of quietly degrading
    // attribution. ~keep
    debug_assert!(
        classification.downgrade_reason.is_some()
            || !(classification.status == SnippetStatus::Downgraded || classification.capability_capped),
        "a Downgraded or capability_capped result must always carry a downgrade_reason"
    );
    let mut result = result(snippet, status, config.level, effective_level, message, duration_ms);
    result.capability_capped = classification.capability_capped;
    result.downgrade_reason = classification.downgrade_reason;
    if let Some(cache) = config.cache_dir.clone().map(ValidationCache::new)
        && let Err(error) = cache.store(
            snippet,
            config.level,
            session.map(|value| value.fingerprint.as_str()),
            &result,
        )
    {
        tracing::warn!("writing snippet validation cache: {error}");
    }
    result
}

fn cached_result(
    snippet: &Snippet,
    config: &RunnerConfig,
    session: Option<&crate::snippets::session::ValidationSession>,
) -> Option<ValidationResult> {
    if !config.changed_only {
        return None;
    }
    let cache = config.cache_dir.clone().map(ValidationCache::new)?;
    let mut result = cache.load(snippet, config.level, session.map(|value| value.fingerprint.as_str()))?;
    result.snippet = snippet.clone();
    result.duration_ms = 0;
    result.message = result.message.or_else(|| Some("cached".to_string()));
    Some(result)
}

fn side_effect_rejection(snippet: &Snippet, config: &RunnerConfig) -> Option<String> {
    if config.level != ValidationLevel::Run {
        return None;
    }
    let Some(class) = snippet.metadata.side_effect else {
        return config
            .deny_unclassified
            .then(|| "unclassified side effects are denied".to_string());
    };
    if class == SideEffectClass::Safe || config.allowed_side_effects.contains(&class) {
        None
    } else {
        Some(format!("side effect class {class:?} is not allowed").to_lowercase())
    }
}

fn result(
    snippet: &Snippet,
    status: SnippetStatus,
    requested_level: ValidationLevel,
    effective_level: ValidationLevel,
    message: Option<String>,
    duration_ms: u64,
) -> ValidationResult {
    ValidationResult {
        snippet: snippet.clone(),
        status,
        level: effective_level,
        requested_level,
        effective_level,
        message,
        duration_ms,
        capability_capped: false,
        downgrade_reason: None,
    }
}

fn skip_message(message: &str, reason: Option<&str>) -> String {
    match reason {
        Some(reason) if !reason.is_empty() => format!("{message}: {reason}"),
        _ => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use crate::snippets::validators::SnippetValidator;
    use std::sync::Arc;
    use tracing_test::traced_test;

    struct RecordingValidator {
        language: crate::snippets::types::Language,
        batches: Arc<Mutex<Vec<(crate::snippets::types::Language, usize, bool)>>>,
        singles: Arc<Mutex<usize>>,
    }

    /// Overruns its timeout on the first call only, then returns immediately. A shared group
    /// deadline is fully consumed by that first call, so any snippet the runner still executes
    /// afterwards proves the budget is per-snippet. ~keep
    #[cfg(unix)]
    struct ExhaustingValidator {
        timeouts: Arc<Mutex<Vec<u64>>>,
    }

    #[cfg(unix)]
    impl SnippetValidator for ExhaustingValidator {
        fn language(&self) -> crate::snippets::types::Language {
            crate::snippets::types::Language::Bash
        }

        fn is_available(&self) -> bool {
            true
        }

        fn validate(
            &self,
            _snippet: &Snippet,
            _level: ValidationLevel,
            timeout_secs: u64,
        ) -> Result<(SnippetStatus, Option<String>)> {
            let call = {
                let mut timeouts = self.timeouts.lock().expect("timeouts");
                timeouts.push(timeout_secs);
                timeouts.len()
            };
            if call == 1 {
                let mut command = std::process::Command::new("sh");
                command.args(["-c", "sleep 30 & wait"]);
                crate::snippets::validators::run_command(&mut command, timeout_secs)?;
            }
            Ok((SnippetStatus::Pass, None))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }
    }

    /// Records the timeout handed to each entry point, and optionally opts in to batching, so a
    /// test can assert which budget a validator actually receives without consuming wall clock.
    struct BudgetRecordingValidator {
        singles: Arc<Mutex<Vec<u64>>>,
        batched: Arc<Mutex<Vec<(usize, u64)>>>,
        supports_batching: bool,
    }

    impl SnippetValidator for BudgetRecordingValidator {
        fn language(&self) -> crate::snippets::types::Language {
            crate::snippets::types::Language::Rust
        }

        fn is_available(&self) -> bool {
            true
        }

        fn validate(
            &self,
            _snippet: &Snippet,
            _level: ValidationLevel,
            timeout_secs: u64,
        ) -> Result<(SnippetStatus, Option<String>)> {
            self.singles.lock().expect("single timeouts").push(timeout_secs);
            Ok((SnippetStatus::Pass, None))
        }

        fn validate_batch_in_session(
            &self,
            snippets: &[&Snippet],
            _level: ValidationLevel,
            timeout_secs: u64,
            _session: Option<&crate::snippets::session::ValidationSession>,
        ) -> Option<Result<Vec<(SnippetStatus, Option<String>)>>> {
            if !self.supports_batching {
                return None;
            }
            self.batched
                .lock()
                .expect("batch timeouts")
                .push((snippets.len(), timeout_secs));
            Some(Ok(vec![(SnippetStatus::Pass, None); snippets.len()]))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }

        fn supports_batching(&self) -> bool {
            self.supports_batching
        }
    }

    impl SnippetValidator for RecordingValidator {
        fn language(&self) -> crate::snippets::types::Language {
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
        ) -> Result<(SnippetStatus, Option<String>)> {
            *self.singles.lock().expect("single count") += 1;
            Ok((SnippetStatus::Pass, None))
        }

        fn validate_batch_in_session(
            &self,
            snippets: &[&Snippet],
            _level: ValidationLevel,
            _timeout_secs: u64,
            session: Option<&crate::snippets::session::ValidationSession>,
        ) -> Option<Result<Vec<(SnippetStatus, Option<String>)>>> {
            self.batches
                .lock()
                .expect("batch records")
                .push((self.language, snippets.len(), session.is_some()));
            Some(Ok(vec![(SnippetStatus::Pass, None); snippets.len()]))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }

        fn supports_batching(&self) -> bool {
            true
        }
    }

    /// A validator that passes but declares a ceiling below `Run`, standing in for the real
    /// zig/toml/json/yaml validators whose maximum level is genuinely lower than TypeCheck.
    struct CappedValidator {
        language: crate::snippets::types::Language,
        ceiling: ValidationLevel,
    }

    impl SnippetValidator for CappedValidator {
        fn language(&self) -> crate::snippets::types::Language {
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
        ) -> Result<(SnippetStatus, Option<String>)> {
            Ok((SnippetStatus::Pass, None))
        }

        fn max_level(&self) -> ValidationLevel {
            self.ceiling
        }
    }

    fn network_snippet() -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: crate::snippets::types::Language::Rust,
            title: None,
            code: "fn main() {}".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata {
                side_effect: Some(SideEffectClass::Network),
                ..SnippetMetadata::default()
            },
            source_origin: SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn side_effect_policy_only_blocks_execution() {
        let snippet = network_snippet();
        let compile = RunnerConfig {
            level: ValidationLevel::Compile,
            ..RunnerConfig::default()
        };
        let run = RunnerConfig {
            level: ValidationLevel::Run,
            ..RunnerConfig::default()
        };

        assert_eq!(side_effect_rejection(&snippet, &compile), None);
        assert_eq!(
            side_effect_rejection(&snippet, &run).as_deref(),
            Some("side effect class network is not allowed")
        );
    }

    #[test]
    fn annotations_cap_validation_instead_of_skipping_it() {
        let mut snippet = network_snippet();
        snippet.annotation = Some(crate::snippets::types::SnippetAnnotation {
            kind: SnippetAnnotationKind::SyntaxOnly,
            reason: None,
        });

        assert_eq!(
            effective_validation_level(&snippet, ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );

        snippet.annotation = None;
        assert_eq!(
            effective_validation_level(&snippet, ValidationLevel::TypeCheck),
            ValidationLevel::TypeCheck
        );
    }

    #[test]
    fn target_session_precedes_canonical_language_fallback() {
        let mut snippet = network_snippet();
        snippet.language = crate::snippets::types::Language::TypeScript;
        snippet.metadata.target = Some("wasm".into());
        let sessions = HashMap::from([
            (
                "typescript".into(),
                crate::snippets::session::ValidationSession {
                    working_directory: "bindings/node".into(),
                    manifest: None,
                    fingerprint: "node".into(),
                    env: Default::default(),
                    include_paths: Vec::new(),
                    rust_features: Vec::new(),
                    rust_dependencies: Default::default(),
                },
            ),
            (
                "wasm".into(),
                crate::snippets::session::ValidationSession {
                    working_directory: "bindings/wasm".into(),
                    manifest: None,
                    fingerprint: "wasm".into(),
                    env: Default::default(),
                    include_paths: Vec::new(),
                    rust_features: Vec::new(),
                    rust_dependencies: Default::default(),
                },
            ),
        ]);

        assert_eq!(
            session_for(&snippet, &sessions).map(|session| session.fingerprint.as_str()),
            Some("wasm")
        );
        snippet.metadata.target = None;
        assert_eq!(
            session_for(&snippet, &sessions).map(|session| session.fingerprint.as_str()),
            Some("node")
        );
    }

    #[test]
    fn groups_batches_by_language_session_and_preserves_order() {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let singles = Arc::new(Mutex::new(0));
        let mut registry = ValidatorRegistry::new();
        for language in [
            crate::snippets::types::Language::Rust,
            crate::snippets::types::Language::Python,
        ] {
            registry.register(Box::new(RecordingValidator {
                language,
                batches: Arc::clone(&batches),
                singles: Arc::clone(&singles),
            }));
        }
        let first_directory = tempfile::tempdir().expect("first session");
        let second_directory = tempfile::tempdir().expect("second session");
        let session = |directory: &std::path::Path| SessionSpec {
            language: crate::snippets::types::Language::Rust,
            working_directory: directory.into(),
            manifest: None,
            before: Vec::new(),
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        };
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            cache_dir: None,
            sessions: HashMap::from([
                ("alpha".into(), session(first_directory.path())),
                ("beta".into(), session(second_directory.path())),
            ]),
            ..RunnerConfig::default()
        };
        let mut snippets = vec![network_snippet(), network_snippet(), network_snippet()];
        snippets[0].id = Some("first".into());
        snippets[0].metadata.target = Some("alpha".into());
        snippets[1].id = Some("second".into());
        snippets[1].metadata.target = Some("beta".into());
        snippets[2].id = Some("third".into());
        snippets[2].language = crate::snippets::types::Language::Python;

        let summary = run_validation(&snippets, &registry, &config).expect("validation succeeds");

        assert_eq!(
            summary
                .results
                .iter()
                .map(|value| value.snippet.id.as_deref())
                .collect::<Vec<_>>(),
            [Some("first"), Some("second"), Some("third")]
        );
        assert_eq!(*singles.lock().expect("single count"), 0);
        let batches = batches.lock().expect("batch records");
        assert_eq!(batches.len(), 3);
        assert!(batches.iter().all(|(_, size, _)| *size == 1));
    }

    #[test]
    fn session_preparation_errors_do_not_abort_healthy_targets() {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let singles = Arc::new(Mutex::new(0));
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(RecordingValidator {
            language: crate::snippets::types::Language::Rust,
            batches: Arc::clone(&batches),
            singles,
        }));
        let directory = tempfile::tempdir().expect("session directory");
        let session = |manifest| SessionSpec {
            language: crate::snippets::types::Language::Rust,
            working_directory: directory.path().into(),
            manifest,
            before: Vec::new(),
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        };
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            cache_dir: None,
            sessions: HashMap::from([
                ("broken".into(), session(Some(directory.path().join("missing.toml")))),
                ("healthy".into(), session(None)),
            ]),
            ..RunnerConfig::default()
        };
        let mut snippets = vec![network_snippet(), network_snippet()];
        snippets[0].metadata.target = Some("broken".into());
        snippets[1].metadata.target = Some("healthy".into());

        let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

        assert_eq!(summary.total, 2);
        assert_eq!(summary.errors, 1);
        assert_eq!(summary.passed, 1);
        assert!(summary.has_failures());
        assert_eq!(summary.results[0].status, SnippetStatus::Error);
        assert!(
            summary.results[0].message.as_deref().is_some_and(
                |message| message.contains("target `broken`") && message.contains("manifest does not exist")
            )
        );
        assert_eq!(summary.results[1].status, SnippetStatus::Pass);
        assert_eq!(
            batches.lock().expect("batch records").as_slice(),
            &[(crate::snippets::types::Language::Rust, 1, true)]
        );
    }

    #[test]
    fn cached_cells_are_excluded_from_batches() {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let singles = Arc::new(Mutex::new(0));
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(RecordingValidator {
            language: crate::snippets::types::Language::Rust,
            batches: Arc::clone(&batches),
            singles,
        }));
        let cache_directory = tempfile::tempdir().expect("cache directory");
        let mut snippets = vec![network_snippet(), network_snippet()];
        snippets[1].code = "fn main() { let _value = 2; }".into();
        let cached = result(
            &snippets[0],
            SnippetStatus::Pass,
            ValidationLevel::Compile,
            ValidationLevel::Compile,
            None,
            1,
        );
        ValidationCache::new(cache_directory.path().into())
            .store(&snippets[0], ValidationLevel::Compile, None, &cached)
            .expect("cache entry");
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            changed_only: true,
            cache_dir: Some(cache_directory.path().into()),
            ..RunnerConfig::default()
        };

        let summary = run_validation(&snippets, &registry, &config).expect("validation succeeds");

        assert_eq!(summary.results.len(), 2);
        assert_eq!(summary.results[0].duration_ms, 0);
        assert_eq!(
            batches.lock().expect("batch records").as_slice(),
            &[(crate::snippets::types::Language::Rust, 1, false)]
        );
    }

    /// One snippet exhausting its timeout must not consume the budget of the next one. The
    /// assertions are on call count and status, never on elapsed wall clock, and the overrun is
    /// 30x the budget so no plausible scheduling delay can flip the outcome. ~keep
    #[cfg(unix)]
    #[test]
    fn a_snippet_that_times_out_does_not_consume_the_next_snippets_budget() {
        let timeouts = Arc::new(Mutex::new(Vec::new()));
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(ExhaustingValidator {
            timeouts: Arc::clone(&timeouts),
        }));
        let mut snippets = vec![network_snippet(), network_snippet()];
        for snippet in &mut snippets {
            snippet.language = crate::snippets::types::Language::Bash;
        }
        let config = RunnerConfig {
            level: ValidationLevel::Run,
            parallelism: 1,
            timeout_secs: 1,
            cache_dir: None,
            // network_snippet() carries SideEffectClass::Network; side_effect_rejection()
            // skips unlisted side effects at ValidationLevel::Run before the validator
            // ever runs (see side_effect_policy_only_blocks_execution), so it must be
            // allow-listed here or the path under test never executes. ~keep
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

        assert_eq!(*timeouts.lock().expect("timeouts"), vec![1, 1]);
        assert_eq!(summary.errors, 1);
        assert_eq!(summary.passed, 1);
        for value in &summary.results {
            let message = value.message.as_deref().unwrap_or_default();
            assert!(
                !message.contains("batch"),
                "reported against a batch command: {message}"
            );
        }
    }

    /// A validator that runs one process per snippet receives the configured timeout for every
    /// snippet, and no snippet is reported against a command the runner never spawned.
    #[test]
    fn non_batching_validators_receive_the_configured_timeout_per_snippet() {
        let singles = Arc::new(Mutex::new(Vec::new()));
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(BudgetRecordingValidator {
            singles: Arc::clone(&singles),
            batched: Arc::new(Mutex::new(Vec::new())),
            supports_batching: false,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            timeout_secs: 7,
            cache_dir: None,
            ..RunnerConfig::default()
        };
        let snippets = vec![network_snippet(), network_snippet(), network_snippet()];

        let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

        assert_eq!(*singles.lock().expect("single timeouts"), vec![7, 7, 7]);
        assert_eq!(summary.passed, 3);
        assert_eq!(summary.errors, 0);
    }

    /// Group budgeting is retained where it is meaningful: a validator that really does cover N
    /// snippets with a single process is handed the whole budget once, not a share of it.
    #[test]
    fn batching_validators_still_receive_the_group_budget_once() {
        let singles = Arc::new(Mutex::new(Vec::new()));
        let batched = Arc::new(Mutex::new(Vec::new()));
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(BudgetRecordingValidator {
            singles: Arc::clone(&singles),
            batched: Arc::clone(&batched),
            supports_batching: true,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            timeout_secs: 7,
            cache_dir: None,
            ..RunnerConfig::default()
        };
        let snippets = vec![network_snippet(), network_snippet(), network_snippet()];

        let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

        assert_eq!(*batched.lock().expect("batch timeouts"), vec![(3, 7)]);
        assert!(singles.lock().expect("single timeouts").is_empty());
        assert_eq!(summary.passed, 3);
    }

    /// A validator whose declared ceiling sits below the requested level has not degraded
    /// anything — that level was never reachable for the language. Marking it `Downgraded`
    /// made `strict` + a level any validator caps below structurally unsatisfiable, so a
    /// consumer's only escape was lowering the level for every other language too. ~keep
    #[test]
    fn validator_ceiling_passes_instead_of_downgrading() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Syntax,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert!(summary.results[0].capability_capped);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
        assert_eq!(
            summary.results[0].downgrade_reason,
            Some(DowngradeReason::ValidatorCapability)
        );
    }

    /// The exemption is narrow: an annotation that lowers the level is the author's choice,
    /// not a capability ceiling, so it must still register as a downgrade and still fail strict. ~keep
    #[test]
    fn annotation_downgrade_is_not_treated_as_a_capability_ceiling() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Run,
        }));
        let mut snippet = network_snippet();
        snippet.annotation = Some(crate::snippets::types::SnippetAnnotation {
            kind: SnippetAnnotationKind::SyntaxOnly,
            reason: None,
        });
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Downgraded);
        assert!(!summary.results[0].capability_capped);
        assert_eq!(summary.downgraded, 1);
        assert_eq!(summary.capability_capped, 0);
        assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Annotation));
    }

    /// A validator whose `max_level` never moves but whose current environment can't back a
    /// deeper level (no real type-checker installed, say) reports that gap through
    /// `achievable_level`, not `max_level`. `php`/`ruby`/`elixir` conflated the two: they claimed
    /// `Run` as their ceiling while their `typecheck`-level check never resolved a symbol, so
    /// `capability_capped` waved every request through as a language-ceiling Pass instead of a
    /// downgrade. This pins the two inputs apart at the mechanism level. ~keep
    struct EnvironmentLimitedValidator {
        language: crate::snippets::types::Language,
    }

    impl SnippetValidator for EnvironmentLimitedValidator {
        fn language(&self) -> crate::snippets::types::Language {
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
        ) -> Result<(SnippetStatus, Option<String>)> {
            Ok((SnippetStatus::Pass, None))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }

        fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
            if requested == ValidationLevel::TypeCheck {
                ValidationLevel::Syntax
            } else {
                ValidationLevel::Run
            }
        }
    }

    #[test]
    fn environment_limited_validator_downgrades_instead_of_capability_capping() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(EnvironmentLimitedValidator {
            language: crate::snippets::types::Language::Rust,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Downgraded);
        assert!(!summary.results[0].capability_capped);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 1);
        assert_eq!(summary.capability_capped, 0);
        assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Environment));
    }

    /// A validator whose `achievable_level` gap is declared structural — see
    /// `achievable_level_is_structural` — is exempted from `Downgraded` the same way a
    /// `max_level` ceiling is, mirroring `validator_ceiling_passes_instead_of_downgrading` but
    /// through the `achievable_level` input instead. This is the generic form of what
    /// `php`/`ruby`/`elixir`/`bash`/`r`'s own tests pin, without depending on a real toolchain. ~keep
    struct StructurallyCappedAchievableValidator {
        language: crate::snippets::types::Language,
    }

    impl SnippetValidator for StructurallyCappedAchievableValidator {
        fn language(&self) -> crate::snippets::types::Language {
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
        ) -> Result<(SnippetStatus, Option<String>)> {
            Ok((SnippetStatus::Pass, None))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }

        fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
            if requested == ValidationLevel::TypeCheck {
                ValidationLevel::Syntax
            } else {
                ValidationLevel::Run
            }
        }

        fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
            requested == ValidationLevel::TypeCheck
        }
    }

    #[test]
    fn structural_achievable_level_gap_is_capability_capped_not_downgraded() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(StructurallyCappedAchievableValidator {
            language: crate::snippets::types::Language::Rust,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert!(summary.results[0].capability_capped);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
        assert_eq!(
            summary.results[0].downgrade_reason,
            Some(DowngradeReason::ValidatorCapability)
        );
    }

    /// The regression this whole change fixes: a front-matter `level:` is a validation contract,
    /// not a suppression. Before this, `discovery::extract_snippets_from_file` collapsed
    /// `metadata.level` into the same `annotation` field a `<!-- snippet:*-only -->` comment
    /// uses, so an author who declared exactly the level they wanted was charged a `Downgraded`
    /// violation identical to one who suppressed validation below what was requested. No test
    /// exercised this end to end through `run_validation` — every prior downgrade test
    /// constructed `snippet.annotation` directly, which is exactly why the collapse went
    /// unnoticed. ~keep
    #[test]
    fn declared_level_contract_passes_instead_of_downgrading() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Run,
        }));
        let mut snippet = network_snippet();
        snippet.metadata.level = Some(ValidationLevel::Syntax);
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert!(!summary.results[0].capability_capped);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 0);
        assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Declared));
        assert_eq!(
            summary.results[0].message.as_deref(),
            Some("requested typecheck, validated at declared level syntax")
        );
    }

    /// Regression for the `docs.snippets.validation_level = "run"` reported as unreachable: an
    /// e2e-generated fixture snippet's front matter always declares `level: typecheck` (see
    /// `e2e::snippets::render_snippet_markdown`), which caps `effective_validation_level` below
    /// any stronger `config.level` a consumer configures — `run` included. That cap is legitimate
    /// (the snippet's own contract, `DowngradeReason::Declared`), so this does not turn it into a
    /// failure; what it must not do is stay silent about the gap. Before the `finalize_result`
    /// message fix, this asserted `"validated at declared level typecheck"`, which never named
    /// what was actually requested — indistinguishable from an ordinary declared-level snippet
    /// with no gap at all. ~keep
    #[test]
    fn declared_typecheck_ceiling_names_the_clamped_run_request() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Run,
        }));
        let mut snippet = network_snippet();
        snippet.metadata.level = Some(ValidationLevel::TypeCheck);
        let config = RunnerConfig {
            level: ValidationLevel::Run,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::TypeCheck);
        assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Declared));
        assert_eq!(
            summary.results[0].message.as_deref(),
            Some("requested run, validated at declared level typecheck"),
            "a `run` request clamped by a snippet's declared level must name both the request and \
             the level it was clamped to, not just the clamped level"
        );
    }

    /// Negative control for the same clamp path: a consumer who legitimately configures a lower
    /// `validation_level` (not `run`) against a snippet with no front-matter `level:` contract at
    /// all must validate normally, with no downgrade classification and no clamp message —
    /// `effective_validation_level` has nothing to fold against `requested`, so it passes through
    /// unchanged. ~keep
    #[test]
    fn legitimately_configured_lower_level_has_no_downgrade_reason() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Run,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::TypeCheck);
        assert_eq!(summary.results[0].downgrade_reason, None);
        assert_eq!(summary.results[0].message, None);
    }

    /// A declared `level:` is a contract for what was requested, not a guarantee the environment
    /// or validator can honor it: when the actual outcome lands below even the declared level,
    /// that is a real downgrade, not a satisfied contract.
    #[test]
    fn declared_level_the_validator_cannot_reach_still_downgrades() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(EnvironmentLimitedValidator {
            language: crate::snippets::types::Language::Rust,
        }));
        let mut snippet = network_snippet();
        snippet.metadata.level = Some(ValidationLevel::Compile);
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[snippet], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Downgraded);
        assert!(!summary.results[0].capability_capped);
        assert_eq!(summary.results[0].effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.results[0].downgrade_reason, Some(DowngradeReason::Environment));
    }

    /// A validator that can reach the requested level must not be flagged at all.
    #[test]
    fn validator_at_or_above_requested_level_is_not_capped() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Run,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::Syntax,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert!(!summary.results[0].capability_capped);
        assert_eq!(summary.capability_capped, 0);
    }

    /// A validator that doesn't support batching at all (every language but rust) must never log
    /// `Starting batched snippet validation` — it never enters that codepath — and its work must
    /// still be observable through the per-snippet fallback's own Starting/Finished pair. Before
    /// `batch_level` checked `supports_batching`, every language was grouped and logged as a
    /// batch regardless, then silently fell through to `validate_one` with no further trace at
    /// all: a `Starting` with no matching `Finished`, and the *real* work invisible. ~keep
    #[traced_test]
    #[test]
    fn non_batching_validator_skips_the_batch_log_and_uses_the_fallback_log() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(CappedValidator {
            language: crate::snippets::types::Language::Rust,
            ceiling: ValidationLevel::Run,
        }));
        let config = RunnerConfig {
            level: ValidationLevel::Syntax,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary =
            run_validation(&[network_snippet(), network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.passed, 2);
        assert!(!logs_contain("Starting batched snippet validation"));
        assert!(logs_contain("Starting per-snippet validation"));
        assert!(logs_contain("Finished per-snippet validation"));
    }

    /// A validator that supports batching in general (rust) can still decline a specific group —
    /// `validate_batch_in_session` returning `None` even though `supports_batching` is `true`,
    /// mirroring rust declining to batch `Run`-level snippets. That group's `Starting batched...`
    /// must resolve to an explicit fallback notice, not a silent `continue` with no matching
    /// `Finished` at all. ~keep
    struct DecliningBatchValidator;

    impl SnippetValidator for DecliningBatchValidator {
        fn language(&self) -> crate::snippets::types::Language {
            crate::snippets::types::Language::Rust
        }

        fn is_available(&self) -> bool {
            true
        }

        fn validate(
            &self,
            _snippet: &Snippet,
            _level: ValidationLevel,
            _timeout_secs: u64,
        ) -> Result<(SnippetStatus, Option<String>)> {
            Ok((SnippetStatus::Pass, None))
        }

        fn validate_batch_in_session(
            &self,
            _snippets: &[&Snippet],
            _level: ValidationLevel,
            _timeout_secs: u64,
            _session: Option<&crate::snippets::session::ValidationSession>,
        ) -> Option<Result<Vec<(SnippetStatus, Option<String>)>>> {
            None
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }

        fn supports_batching(&self) -> bool {
            true
        }
    }

    #[traced_test]
    #[test]
    fn batching_validator_that_declines_a_group_logs_the_fallback_explicitly() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(DecliningBatchValidator));
        let config = RunnerConfig {
            level: ValidationLevel::Syntax,
            parallelism: 1,
            cache_dir: None,
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[network_snippet()], &registry, &config).expect("validation completes");

        assert_eq!(summary.results[0].status, SnippetStatus::Pass);
        assert!(logs_contain("Starting batched snippet validation"));
        assert!(logs_contain(
            "Batch validation declined for this group; falling back to per-snippet validation"
        ));
        assert!(logs_contain("Starting per-snippet validation"));
    }

    /// Always reports the same failure, standing in for a language failing at 100% — the shape of
    /// the run that produced 1,753 failures with no log output until the stage ended.
    struct FailingValidator;

    impl SnippetValidator for FailingValidator {
        fn language(&self) -> crate::snippets::types::Language {
            crate::snippets::types::Language::Java
        }

        fn is_available(&self) -> bool {
            true
        }

        fn validate(
            &self,
            _snippet: &Snippet,
            _level: ValidationLevel,
            _timeout_secs: u64,
        ) -> Result<(SnippetStatus, Option<String>)> {
            Ok((
                SnippetStatus::Fail,
                Some("Example.java:1: error: duplicate class: Example\n  1 error".into()),
            ))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
        }
    }

    fn failing_snippet(language: crate::snippets::types::Language) -> Snippet {
        let mut snippet = network_snippet();
        snippet.language = language;
        snippet
    }

    fn failure_result(snippet: &Snippet, message: &str) -> ValidationResult {
        result(
            snippet,
            SnippetStatus::Fail,
            ValidationLevel::Compile,
            ValidationLevel::Compile,
            Some(message.to_string()),
            1,
        )
    }

    /// The defect: 1,753 snippet failures went straight to the result cache and the final summary,
    /// so six languages failing at 100% were indistinguishable from a healthy run for the entire
    /// stage. What makes "before the stage ends" checkable here is the *absence* of the terminal
    /// per-language event: two of this language's three snippets are still outstanding, so the
    /// summary cannot have run, yet the first failure and its validator message are already out. ~keep
    #[traced_test]
    #[test]
    fn a_languages_first_failure_is_reported_before_its_stage_ends() {
        let snippets = vec![
            failing_snippet(crate::snippets::types::Language::Java),
            failing_snippet(crate::snippets::types::Language::Java),
            failing_snippet(crate::snippets::types::Language::Java),
        ];
        let reporter = FailureReporter::new(&snippets);

        reporter.record(&failure_result(
            &snippets[0],
            "error: cannot find symbol\n  symbol: class Missing",
        ));

        assert!(logs_contain("First snippet validation failure for this language"));
        assert!(logs_contain("cannot find symbol | symbol: class Missing"));
        assert!(!logs_contain("Finished snippet validation for this language"));
    }

    /// The other half of the requirement: visible, but not a firehose. Failure two emits nothing
    /// at all, and a running count only appears once the stride is reached — so a 1,753-failure
    /// run costs tens of lines, not 1,753. ~keep
    #[traced_test]
    #[test]
    fn failures_after_the_first_are_counted_rather_than_logged_one_line_each() {
        let snippets = vec![failing_snippet(crate::snippets::types::Language::Java); FAILURE_PROGRESS_STRIDE + 1];
        let reporter = FailureReporter::new(&snippets);

        for snippet in snippets.iter().take(2) {
            reporter.record(&failure_result(snippet, "compilation failed"));
        }
        assert!(logs_contain("First snippet validation failure for this language"));
        assert!(!logs_contain("Snippet validation failures accumulating"));

        for snippet in snippets.iter().take(FAILURE_PROGRESS_STRIDE).skip(2) {
            reporter.record(&failure_result(snippet, "compilation failed"));
        }
        assert!(logs_contain("Snippet validation failures accumulating"));
        assert!(!logs_contain("Finished snippet validation for this language"));
    }

    /// The reporter must be wired into the real per-snippet dispatch path, not just constructible:
    /// `parallel_results` is where all 1,753 failures were produced and dropped on the floor.
    #[traced_test]
    #[test]
    fn a_failing_language_is_reported_through_the_real_validation_run() {
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(FailingValidator));
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };
        let snippets = vec![
            failing_snippet(crate::snippets::types::Language::Java),
            failing_snippet(crate::snippets::types::Language::Java),
        ];

        let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

        assert_eq!(summary.failed, 2);
        assert!(logs_contain("First snippet validation failure for this language"));
        assert!(logs_contain("duplicate class: Example"));
        assert!(logs_contain(
            "Finished snippet validation for this language with failures"
        ));
    }

    #[test]
    fn a_failure_preview_is_a_single_bounded_line() {
        let long = "x".repeat(FAILURE_MESSAGE_PREVIEW_CHARS + 50);
        let preview = failure_preview(Some(long.as_str()));
        assert_eq!(preview.len(), FAILURE_MESSAGE_PREVIEW_CHARS + 3);
        assert!(preview.ends_with("..."));
        assert_eq!(failure_preview(Some("  \n\n ")), "<no validator output>");
        assert_eq!(failure_preview(None), "<no validator output>");
        assert_eq!(failure_preview(Some("first\n\nsecond")), "first | second");
    }
}
