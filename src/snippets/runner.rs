use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::session::{SessionSpec, prepare_sessions};
use crate::snippets::types::{
    RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel, ValidationResult,
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
    let sessions = prepare_sessions(&config.sessions, config.timeout_secs)?;
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
                let session = session_for(snippet, &sessions);
                let lock = session_key(snippet, &sessions).and_then(|key| session_locks.get(key));
                let result = validate_one(snippet, registry, config, session, lock);
                let should_stop = matches!(result.status, SnippetStatus::Fail | SnippetStatus::Error);
                results.push(result);
                if should_stop {
                    break;
                }
            }
            results
        } else {
            let batched = validate_batches(snippets, registry, config, &sessions, &session_locks);
            snippets
                .par_iter()
                .enumerate()
                .map(|(index, snippet)| {
                    if let Some(result) = batched[index].clone() {
                        return result;
                    }
                    let session = session_for(snippet, &sessions);
                    let lock = session_key(snippet, &sessions).and_then(|key| session_locks.get(key));
                    validate_one(snippet, registry, config, session, lock)
                })
                .collect()
        }
    });

    Ok(RunSummary::from_results(results))
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
    session_locks: &HashMap<String, Mutex<()>>,
) -> Vec<Option<ValidationResult>> {
    let mut results = vec![None; snippets.len()];
    let mut groups = BTreeMap::<BatchKey, Vec<usize>>::new();
    for (index, snippet) in snippets.iter().enumerate() {
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

fn validate_one(
    snippet: &Snippet,
    registry: &ValidatorRegistry,
    config: &RunnerConfig,
    session: Option<&crate::snippets::session::ValidationSession>,
    session_lock: Option<&Mutex<()>>,
) -> ValidationResult {
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

    if status == SnippetStatus::Pass && effective_level < config.level {
        status = SnippetStatus::Downgraded;
    }
    let message = if status == SnippetStatus::Downgraded {
        Some(format!("requested {}, validated at {}", config.level, effective_level))
    } else {
        message
    };
    let result = result(snippet, status, config.level, effective_level, message, duration_ms);
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
}
