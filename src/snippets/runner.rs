use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::session::{SessionSpec, prepare_sessions_isolated};
use crate::snippets::types::{
    RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel, ValidationResult,
};
use crate::snippets::validators::ValidatorRegistry;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    let results: Vec<ValidationResult> = pool.install(|| {
        if fail_fast {
            let mut results = Vec::with_capacity(snippets.len());
            for snippet in snippets {
                let preparation_error = session_preparation_error(snippet, &sessions, &session_errors);
                let session = session_for(snippet, &sessions);
                let lock = session_key(snippet, &sessions).and_then(|key| session_locks.get(key));
                let result = validate_one(snippet, registry, config, session, lock, preparation_error, None);
                let should_stop =
                    preparation_error.is_none() && matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
                results.push(result);
                if should_stop {
                    break;
                }
            }
            results
        } else {
            let batched = validate_batches(snippets, registry, config, &sessions, &session_errors, &session_locks);
            let batch_deadlines = snippets
                .iter()
                .enumerate()
                .filter(|(index, _)| batched[*index].is_none())
                .filter_map(|(_, snippet)| {
                    validation_batch_key(snippet, registry, config, &sessions).map(|key| (key, OnceLock::new()))
                })
                .collect::<BTreeMap<_, _>>();
            snippets
                .par_iter()
                .enumerate()
                .map(|(index, snippet)| {
                    if let Some(result) = batched[index].clone() {
                        return result;
                    }
                    let session = session_for(snippet, &sessions);
                    let lock = session_key(snippet, &sessions).and_then(|key| session_locks.get(key));
                    let batch_started = validation_batch_key(snippet, registry, config, &sessions)
                        .and_then(|key| batch_deadlines.get(&key));
                    validate_one(snippet, registry, config, session, lock, None, batch_started)
                })
                .collect()
        }
    });

    Ok(RunSummary::from_results(results))
}

type BatchKey = (crate::snippets::types::Language, Option<String>, ValidationLevel);

fn validation_batch_key(
    snippet: &Snippet,
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    sessions: &HashMap<String, crate::snippets::session::ValidationSession>,
) -> Option<BatchKey> {
    let session = session_for(snippet, sessions);
    batch_level(snippet, registry, config, session).map(|level| {
        (
            snippet.language,
            session_key(snippet, sessions).map(str::to_string),
            level,
        )
    })
}

fn remaining_batch_timeout(started: &OnceLock<Instant>, timeout_secs: u64) -> u64 {
    let deadline = *started.get_or_init(Instant::now) + Duration::from_secs(timeout_secs);
    deadline
        .checked_duration_since(Instant::now())
        // Round the remainder up to whole seconds instead of truncating: `Duration::as_secs`
        // floors, so a budget with only nanoseconds elapsed (the common case for the first
        // caller right after `get_or_init`) truncates to 0 and starves every validator in the
        // batch before any of them run. Rounding up keeps the shared deadline meaningful at
        // whole-second granularity without ever reporting time left as none. ~keep
        .map(|remaining| remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0))
        .unwrap_or(0)
}

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
) -> Vec<Option<ValidationResult>> {
    let mut results = vec![None; snippets.len()];
    let mut groups = BTreeMap::<BatchKey, Vec<usize>>::new();
    for (index, snippet) in snippets.iter().enumerate() {
        if let Some(message) = session_preparation_error(snippet, sessions, session_errors) {
            results[index] = Some(result(
                snippet,
                SnippetStatus::Error,
                config.level,
                config.level,
                Some(message.to_owned()),
                0,
            ));
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
        let Some(batch) = batch else {
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
            results[index] = Some(finalize_result(
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
            ));
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
    let level = effective_validation_level(snippet, config.level).min(validator.max_level());
    validator.is_available_at(level).then_some(level)
}

fn effective_validation_level(snippet: &Snippet, requested: ValidationLevel) -> ValidationLevel {
    let limit = snippet
        .annotation
        .as_ref()
        .and_then(|annotation| match annotation.kind {
            SnippetAnnotationKind::SyntaxOnly => Some(ValidationLevel::Syntax),
            SnippetAnnotationKind::CompileOnly => Some(ValidationLevel::Compile),
            SnippetAnnotationKind::TypeCheckOnly => Some(ValidationLevel::TypeCheck),
            SnippetAnnotationKind::Skip => None,
        });
    limit.map_or(requested, |level| requested.min(level))
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
    batch_started: Option<&OnceLock<Instant>>,
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

    if let Some(annotation) = &snippet.annotation {
        match annotation.kind {
            SnippetAnnotationKind::Skip => {
                return result(
                    snippet,
                    SnippetStatus::Skip,
                    config.level,
                    config.level,
                    Some(skip_message("skipped via annotation", annotation.reason.as_deref())),
                    0,
                );
            }
            _ => {}
        }
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

    let effective_level = effective_validation_level(snippet, config.level).min(validator.max_level());
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
    let validation = || {
        let timeout_secs = batch_started.map_or(config.timeout_secs, |started| {
            remaining_batch_timeout(started, config.timeout_secs)
        });
        if timeout_secs == 0 {
            return Err(crate::snippets::error::Error::Timeout {
                command: format!("{} validation batch", snippet.language),
                timeout_secs: config.timeout_secs,
            });
        }
        validator.validate_in_session(snippet, effective_level, timeout_secs, session)
    };
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

    // A reduction caused only by the validator's declared `max_level` is a capability ceiling,
    // not a degraded run: that level was never reachable for this language, so counting it as a
    // downgrade makes a strict request for it unsatisfiable however healthy the environment is.
    // Annotation-driven reductions and environmental failures are deliberately unaffected. ~keep
    let annotated_level = effective_validation_level(snippet, config.level);
    let capability_capped = status == SnippetStatus::Pass
        && effective_level < config.level
        && annotated_level >= config.level
        && validator.max_level() < config.level;

    if status == SnippetStatus::Pass && effective_level < config.level && !capability_capped {
        status = SnippetStatus::Downgraded;
    }
    let message = if status == SnippetStatus::Downgraded {
        Some(format!("requested {}, validated at {}", config.level, effective_level))
    } else if capability_capped {
        Some(format!(
            "requested {}, validated at {} ({} validator caps at {})",
            config.level,
            effective_level,
            snippet.language,
            validator.max_level()
        ))
    } else {
        message
    };
    let mut result = result(snippet, status, config.level, effective_level, message, duration_ms);
    result.capability_capped = capability_capped;
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

    struct RecordingValidator {
        language: crate::snippets::types::Language,
        batches: Arc<Mutex<Vec<(crate::snippets::types::Language, usize, bool)>>>,
        singles: Arc<Mutex<usize>>,
    }

    #[cfg(unix)]
    struct TimeoutValidator {
        calls: Arc<Mutex<usize>>,
    }

    #[cfg(unix)]
    impl SnippetValidator for TimeoutValidator {
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
            *self.calls.lock().expect("call count") += 1;
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "sleep 30 & wait"]);
            crate::snippets::validators::run_command(&mut command, timeout_secs)?;
            Ok((SnippetStatus::Pass, None))
        }

        fn max_level(&self) -> ValidationLevel {
            ValidationLevel::Run
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

    #[cfg(unix)]
    #[test]
    fn timeout_is_shared_by_all_snippets_in_a_validation_batch() {
        let calls = Arc::new(Mutex::new(0));
        let mut registry = ValidatorRegistry::new();
        registry.register(Box::new(TimeoutValidator {
            calls: Arc::clone(&calls),
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
            // allow-listed here or the budget-sharing path under test never executes. ~keep
            allowed_side_effects: vec![SideEffectClass::Network],
            ..RunnerConfig::default()
        };

        let started = Instant::now();
        let summary = run_validation(&snippets, &registry, &config).expect("validation completes");

        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(*calls.lock().expect("call count"), 1);
        assert_eq!(summary.errors, 2);
        assert!(summary.results.iter().all(|value| {
            value
                .message
                .as_deref()
                .is_some_and(|message| message.contains("timed out after 1s"))
        }));
    }

    // Timing-safe regression coverage for the truncation bug directly: instead of racing a
    // real sleep against a wall-clock threshold, an already-elapsed `Instant` is seeded into
    // the `OnceLock` so the function is exercised deterministically at a fixed, known offset. ~keep
    #[test]
    fn remaining_batch_timeout_rounds_up_a_still_live_budget() {
        let started = OnceLock::new();
        started
            .set(Instant::now() - Duration::from_millis(1))
            .expect("OnceLock starts empty");

        assert_eq!(remaining_batch_timeout(&started, 1), 1);
    }

    #[test]
    fn remaining_batch_timeout_reports_zero_once_the_deadline_has_passed() {
        let started = OnceLock::new();
        started
            .set(Instant::now() - Duration::from_secs(2))
            .expect("OnceLock starts empty");

        assert_eq!(remaining_batch_timeout(&started, 1), 0);
    }

    /// A validator whose declared ceiling sits below the requested level has not degraded
    /// anything — that level was never reachable for the language. Marking it `Downgraded`
    /// made `strict` + a level any validator caps below structurally unsatisfiable, so a
    /// consumer's only escape was lowering the level for every other language too.
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
    }

    /// The exemption is narrow: an annotation that lowers the level is the author's choice,
    /// not a capability ceiling, so it must still register as a downgrade and still fail strict.
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
}
