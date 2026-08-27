use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::session::{SessionSpec, prepare_sessions_isolated};
use crate::snippets::types::{
    DowngradeReason, RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel,
    ValidationResult,
};
use crate::snippets::validators::ValidatorRegistry;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

mod batch;
mod dependency_reclassification;
mod session_locks;
mod session_prep;
mod session_resolution;

use batch::validate_batches;
use session_locks::{session_lock_for, session_locks_by_fingerprint};
use session_prep::{session_preparation_error, session_preparation_result};

// Re-exported so `output::unresolved_dependency_rollup` can split its two remediation lines
// without a new serialized `ValidationResult` field, and so its tests can build a realistic
// fixture message -- see `dependency_reclassification`'s module doc. ~keep
pub(crate) use dependency_reclassification::{NO_SESSION_CONFIGURED_PHRASE, unresolved_dependency_message};

pub struct RunnerConfig {
    pub level: ValidationLevel,
    pub parallelism: usize,
    pub timeout_secs: u64,
    /// The budget for a session's `before` hook, when it must differ from `timeout_secs`.
    ///
    /// A hook builds a whole package -- `./gradlew assembleDebug`, `pnpm run build:all` -- while
    /// `timeout_secs` bounds one snippet's compiler invocation. Sharing one number means the only
    /// way to give a cold build the minutes it needs is to give every snippet compile the same
    /// minutes, which is how a runaway hook got a half-hour ceiling to run out. `None` keeps the
    /// single-number behaviour. ~keep
    pub before_timeout_secs: Option<u64>,
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
            before_timeout_secs: None,
            fail_fast: false,
            deny_unclassified: false,
            allowed_side_effects: Vec::new(),
            cache_dir: Some(std::path::PathBuf::from(".alef/snippets")),
            changed_only: false,
            sessions: HashMap::new(),
        }
    }
}

impl RunnerConfig {
    /// The budget a session's `before` hook actually runs under.
    #[must_use]
    pub fn resolved_before_timeout_secs(&self) -> u64 {
        self.before_timeout_secs.unwrap_or(self.timeout_secs)
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
    let sessions_to_prepare = sessions_needed_for_preparation(snippets, &config.sessions);
    let preparation = prepare_sessions_isolated(&sessions_to_prepare, config.resolved_before_timeout_secs());
    let sessions = preparation.sessions;
    let session_errors = preparation.errors;
    // Keyed by *fingerprint*, not by the config session name: `alef.toml` can point two
    // differently-named sessions (a language fallback like `typescript` and an explicit
    // binding-package target like `node`) at the identical `cwd`/manifest, which resolves to one
    // physical workspace directory. Keying by name handed each alias its own `Mutex`, so two
    // batch groups that both believed they held "the" session lock wrote into the same
    // `snippet_batch_N.ts` files concurrently -- see `session_lock_for`. ~keep
    let session_locks = session_locks_by_fingerprint(sessions.values());
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
    session_errors: &HashMap<String, crate::snippets::session::SessionPreparationError>,
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
        let preparation_error = session_preparation_error(snippet, config, session_errors);
        let session = session_for(snippet, sessions);
        let lock = session_lock_for(session, session_locks);
        let result = validate_one(
            snippet,
            registry,
            config,
            session,
            lock,
            preparation_error.as_ref(),
            Some(&reporter),
        );
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
    session_errors: &HashMap<String, crate::snippets::session::SessionPreparationError>,
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
    let unclaimed_counts = fallback_counts_by_language(snippets, &batched);
    let started = Instant::now();
    let results = snippets
        .par_iter()
        .enumerate()
        .map(|(index, snippet)| {
            if let Some(result) = batched[index].clone() {
                return result;
            }
            let session = session_for(snippet, sessions);
            let lock = session_lock_for(session, session_locks);
            let result = validate_one(snippet, registry, config, session, lock, None, Some(&reporter));
            reporter.record(&result);
            result
        })
        .collect();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    for (language, invoked) in reporter.invoked_by_language() {
        // ~keep `resolved_without_toolchain` is the gap between what the batch pass declined to
        // claim and what actually ran: cache hits, `skip` annotations, side-effect rejections and
        // unavailable toolchains. Reporting it beside `snippet_count` keeps the two from being
        // conflated again -- reading the unclaimed count *as* the validated count is what made a
        // fully-cached language look like it was validating 521 snippets one at a time.
        let unclaimed = unclaimed_counts.get(&language).copied().unwrap_or(invoked);
        tracing::info!(
            language = %language,
            snippet_count = invoked,
            resolved_without_toolchain = unclaimed.saturating_sub(invoked),
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
    unavailable: usize,
    /// Snippets of this language that actually reached a toolchain invocation, as counted by
    /// [`FailureReporter::record_toolchain_start`]. ~keep Distinct from `completed`, which counts
    /// every result including the ones `validate_one` short-circuits without running anything.
    invoked: usize,
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

    /// Announce, once per language, that a snippet of that language has reached an actual
    /// toolchain invocation, and count every such invocation.
    ///
    /// ~keep This is called from the one place real work begins, rather than inferred beforehand
    /// from `validate_batches` leaving an entry unclaimed. That inference was wrong: `validate_one`
    /// short-circuits on a cache hit, a `skip` annotation, a side-effect rejection, a missing
    /// validator and an unavailable toolchain, none of which run anything. Because
    /// `docs::validate_snippets` runs once per crate with `changed_only`, later crates are ~100%
    /// cache hits, so every language announced `Starting per-snippet validation snippet_count=521`
    /// while doing nothing at all -- which read as though 13 of 14 languages had fallen out of
    /// batching when in fact only four had. Deriving the event from the work itself cannot drift
    /// from it the way a parallel predicate can.
    fn record_toolchain_start(&self, language: crate::snippets::types::Language, timeout_secs: u64) {
        let Ok(mut tallies) = self.tallies.lock() else {
            return;
        };
        let tally = tallies.entry(language).or_default();
        tally.invoked += 1;
        let first = tally.invoked == 1;
        drop(tallies);
        if !first {
            return;
        }
        let snippet_count = self.totals.get(&language).copied().unwrap_or(0);
        self.span.in_scope(|| {
            tracing::info!(
                language = %language,
                snippet_count = snippet_count,
                timeout_secs = timeout_secs,
                "Starting per-snippet validation"
            );
        });
    }

    /// Per-language counts of snippets that actually invoked a toolchain.
    fn invoked_by_language(&self) -> BTreeMap<crate::snippets::types::Language, usize> {
        let Ok(tallies) = self.tallies.lock() else {
            return BTreeMap::new();
        };
        tallies
            .iter()
            .filter(|(_, tally)| tally.invoked > 0)
            .map(|(language, tally)| (*language, tally.invoked))
            .collect()
    }

    fn record(&self, result: &ValidationResult) {
        let language = result.snippet.language;
        let failed = matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
        // `Unavailable` is tallied separately rather than folded into `failed`, and it is tallied at
        // all because it is not the harmless outcome its name suggests: under `strict` it fails the
        // run exactly like a `Fail`, and the `unresolved_dependency` reclassification below turns a
        // real validator `Fail` -- diagnostic and all -- into one. Counting only `Fail | Error` is
        // how 566 snippets across two languages reached the final summary as
        // "283 unresolved dependency" apiece with not one line anywhere in the log saying WHICH
        // dependency, while the validator's own message sat unread on every result. ~keep
        let unavailable = matches!(result.status, SnippetStatus::Unavailable);
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
        if unavailable {
            tally.unavailable += 1;
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
            } else if unavailable && tally.unavailable == 1 {
                tracing::warn!(
                    language = %language,
                    path = %result.snippet.source_origin.path.display(),
                    line = result.snippet.source_origin.line,
                    snippet_count = snippet_count,
                    unresolved_dependency = result.unresolved_dependency,
                    error = %failure_preview(result.message.as_deref()),
                    "First snippet validation unavailability for this language"
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
                    unavailable = tally.unavailable,
                    snippet_count = snippet_count,
                    "Finished snippet validation for this language with failures"
                );
            } else if tally.unavailable > 0 {
                tracing::warn!(
                    language = %language,
                    unavailable = tally.unavailable,
                    snippet_count = snippet_count,
                    "Finished snippet validation for this language with every result unvalidated"
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

/// The subset of `all_sessions` this run actually needs prepared: every session a snippet in
/// `snippets` resolves to (via `session_resolution::resolve_session_claim`, run against the
/// *full* configured map so ambiguity detection is unaffected downstream), expanded to include
/// any other configured session that shares a `working_directory` with one of those.
///
/// Preparing only the sessions a filtered run (`alef snippets check --lang go`) actually touches
/// is the whole point: unfiltered, every configured `before` hook ran regardless of `--lang`,
/// because `prepare_sessions_isolated` was always handed the complete `config.sessions` map --
/// discovery narrowed to one language while session preparation stayed crate-wide, which is what
/// turned a single-language diagnostic into an hour-plus run across every configured toolchain.
///
/// Dropping an unneeded session from the map handed to `prepare_sessions_isolated` is not free,
/// though: `session::purge_stale_session_scratch` sweeps a working directory's scratch root down
/// to only the fingerprints *this call* resolved, and two sessions can legitimately share one
/// working directory. Omitting a cohabiting session here would make its live fingerprint
/// directory look abandoned and delete it -- destroying a sibling session's build cache to speed
/// up an unrelated run. The expansion step exists solely to prevent that: any session sharing a
/// working directory with a needed one is kept too, even though no snippet in this run claims it
/// directly. ~keep
fn sessions_needed_for_preparation(
    snippets: &[Snippet],
    all_sessions: &HashMap<String, SessionSpec>,
) -> HashMap<String, SessionSpec> {
    let mut needed: HashSet<&str> = HashSet::new();
    for snippet in snippets {
        match session_resolution::resolve_session_claim(snippet, all_sessions, |spec| spec.language) {
            session_resolution::SessionClaim::Claimed(key) => {
                needed.insert(key);
            }
            session_resolution::SessionClaim::Ambiguous(candidates) => {
                needed.extend(candidates);
            }
            session_resolution::SessionClaim::Unclaimed => {}
        }
    }

    let shared_directories: HashSet<&std::path::Path> = all_sessions
        .iter()
        .filter(|(key, _)| needed.contains(key.as_str()))
        .map(|(_, spec)| spec.working_directory.as_path())
        .collect();

    all_sessions
        .iter()
        .filter(|(key, spec)| {
            needed.contains(key.as_str()) || shared_directories.contains(spec.working_directory.as_path())
        })
        .map(|(key, spec)| (key.clone(), spec.clone()))
        .collect()
}

/// The single resolution `fail_fast_results`, `parallel_results` and
/// `batch::group_batchable_snippets` all share for "which configured session does this snippet
/// use" -- see `session_resolution` for why the resolver, not a per-caller string lookup, is what
/// keeps a snippet's session and its batch key from ever disagreeing. ~keep
fn session_for<'a>(
    snippet: &Snippet,
    sessions: &'a HashMap<String, crate::snippets::session::ValidationSession>,
) -> Option<&'a crate::snippets::session::ValidationSession> {
    match session_resolution::resolve_session_claim(snippet, sessions, |session| session.language) {
        session_resolution::SessionClaim::Claimed(key) => sessions.get(key),
        session_resolution::SessionClaim::Unclaimed | session_resolution::SessionClaim::Ambiguous(_) => None,
    }
}

fn session_key<'a>(
    snippet: &Snippet,
    sessions: &'a HashMap<String, crate::snippets::session::ValidationSession>,
) -> Option<&'a str> {
    match session_resolution::resolve_session_claim(snippet, sessions, |session| session.language) {
        session_resolution::SessionClaim::Claimed(key) => Some(key),
        session_resolution::SessionClaim::Unclaimed | session_resolution::SessionClaim::Ambiguous(_) => None,
    }
}

fn validate_one(
    snippet: &Snippet,
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    session: Option<&crate::snippets::session::ValidationSession>,
    session_lock: Option<&Mutex<()>>,
    session_preparation_error: Option<&crate::snippets::session::SessionPreparationError>,
    reporter: Option<&FailureReporter>,
) -> ValidationResult {
    if let Some(preparation_error) = session_preparation_error {
        return session_preparation_result(snippet, config, preparation_error);
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

    if let Some(reporter) = reporter {
        reporter.record_toolchain_start(snippet.language, config.timeout_secs);
    }

    // `timeout_secs` is a per-invocation budget here, not a group budget. This path runs one
    // toolchain process per snippet (every validator except rust's non-`Run` batch), so sharing a
    // single wall-clock deadline across the language group let the first snippet consume it and
    // left the rest reported as toolchain timeouts for commands that were never spawned. Group
    // budgeting belongs to `validate_batches`, where one process really does cover N snippets. ~keep
    // ~keep `start` is taken *inside* the session lock, not before it. Taken outside, every
    // recorded `duration_ms` on this path included time spent queueing behind other snippets of
    // the same session, which serializes here -- zig snippets doing ~5.9s of real work were
    // recorded at a 58s median, making the per-invocation cost look ~10x worse than it is and
    // hiding the serialization behind it. This measures the toolchain, and only the toolchain.
    let mut start = Instant::now();
    let validation = |start: &mut Instant| {
        *start = Instant::now();
        validator.validate_in_session(snippet, effective_level, config.timeout_secs, session)
    };
    // ~keep Only validators that share fixed-name files inside the session workspace need the
    // mutex; see `SnippetValidator::requires_session_exclusivity`. Taking it for everyone made
    // this path strictly serial per session even though it runs inside a rayon pool, which is why
    // 521 zig snippets of ~5.9s each took half an hour.
    let session_lock = session_lock.filter(|_| validator.requires_session_exclusivity());
    let validation_result = match session_lock {
        Some(lock) => match lock.lock() {
            Ok(_guard) => validation(&mut start),
            Err(error) => Err(crate::snippets::error::Error::Other(format!(
                "locking {} snippet validation session: {error}",
                snippet.language
            ))),
        },
        None => validation(&mut start),
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
    // A validator's toolchain can run to completion and still report a `Fail` whose message is a
    // missing import/package/symbol rather than a defect in the snippet — the shape every
    // `is_dependency_error` implementation recognizes. Below `Compile` (i.e. `Syntax`), that is
    // expected: syntax checking was never supposed to resolve anything, so it stays a `Pass`. At
    // `Compile`/`TypeCheck`/`Run`, it means this run's environment could not back the validation
    // it just attempted — either because `alef build` never produced the artifact the snippet
    // links or imports against (reported by `docs::enforce_snippet_summary`, once validation has
    // actually run and can say so with evidence), or because no session is configured for this
    // language at all, so it never had a manifest to
    // resolve against in the first place -- see `dependency_reclassification` for that split.
    // Reported as `Unavailable` with `unresolved_dependency` set, not `Fail`: a `Fail` here would
    // be indistinguishable from a genuine emitter bug, which is exactly the defect this
    // reclassification exists to close. ~keep
    let mut unresolved_dependency = false;
    if status == SnippetStatus::Fail
        && let Some(error_output) = &message
        && validator.is_dependency_error(error_output)
    {
        if effective_level == ValidationLevel::Syntax {
            status = SnippetStatus::Pass;
        } else {
            status = SnippetStatus::Unavailable;
            unresolved_dependency = true;
        }
    }
    // See `dependency_reclassification`'s module doc: by the time this runs, `session` is `None`
    // only when no configured `docs.snippets.sessions` target claims this snippet's language --
    // every claimed session that failed to prepare already short-circuited through
    // `session_prep::session_preparation_result` before a validator ever ran. That makes this an
    // unambiguous signal that `alef build` cannot fix the result, unlike the `Some(session)` case
    // where a session exists and the artifact is simply not built yet. ~keep
    let no_session_configured = unresolved_dependency && session.is_none();

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
    } else if unresolved_dependency {
        // See task #470 and `dependency_reclassification::session_target_hint`'s doc comment: the
        // actionable config-key hint must name the snippet's own resolved target, not its
        // language.
        Some(unresolved_dependency_message(
            no_session_configured,
            snippet.language,
            &dependency_reclassification::session_target_hint(snippet),
            effective_level,
            message.as_deref().unwrap_or("<no validator output>"),
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
    // Bounded only here, after `is_dependency_error` and the reclassification wording have both
    // read the validator's untruncated output: from this point the message is prose, printed and
    // serialized, never matched against. ~keep
    let message = message.map(|message| crate::snippets::diagnostics::bounded_text(&message));
    let mut result = result(snippet, status, config.level, effective_level, message, duration_ms);
    result.capability_capped = classification.capability_capped;
    result.downgrade_reason = classification.downgrade_reason;
    result.unresolved_dependency = unresolved_dependency;
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

pub(super) fn result(
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
        unresolved_dependency: false,
    }
}

fn skip_message(message: &str, reason: Option<&str>) -> String {
    match reason {
        Some(reason) if !reason.is_empty() => format!("{message}: {reason}"),
        _ => message.to_string(),
    }
}

#[cfg(test)]
mod no_work_logging_tests;

#[cfg(test)]
mod session_concurrency_tests;

#[cfg(test)]
mod session_preparation_classification_tests;

#[cfg(test)]
mod validation_dispatch_tests;

#[cfg(test)]
mod downgrade_classification_tests;

#[cfg(test)]
mod batch_logging_tests;

#[cfg(test)]
mod failure_reporting_tests;

#[cfg(test)]
mod session_scope_tests;
