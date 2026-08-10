use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::session::{SessionSpec, prepare_sessions};
use crate::snippets::types::{
    RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel, ValidationResult,
};
use crate::snippets::validators::ValidatorRegistry;
use rayon::prelude::*;
use std::collections::HashMap;
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
            snippets
                .par_iter()
                .map(|snippet| {
                    let session = session_for(snippet, &sessions);
                    let lock = session_key(snippet, &sessions).and_then(|key| session_locks.get(key));
                    validate_one(snippet, registry, config, session, lock)
                })
                .collect()
        }
    });

    Ok(RunSummary::from_results(results))
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
            SnippetAnnotationKind::SyntaxOnly if config.level > ValidationLevel::Syntax => {
                return result(
                    snippet,
                    SnippetStatus::Skip,
                    config.level,
                    ValidationLevel::Syntax,
                    Some("annotation limits to syntax-only".to_string()),
                    0,
                );
            }
            SnippetAnnotationKind::CompileOnly if config.level > ValidationLevel::Compile => {
                return result(
                    snippet,
                    SnippetStatus::Skip,
                    config.level,
                    ValidationLevel::Compile,
                    Some("annotation limits to compile-only".to_string()),
                    0,
                );
            }
            SnippetAnnotationKind::TypeCheckOnly if config.level > ValidationLevel::TypeCheck => {
                return result(
                    snippet,
                    SnippetStatus::Skip,
                    config.level,
                    ValidationLevel::TypeCheck,
                    Some("annotation limits to typecheck-only".to_string()),
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

    let effective_level = config.level.min(validator.max_level());
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
    let (mut status, message) = match validation_result {
        Ok((status, message)) => (status, message),
        Err(err) => (SnippetStatus::Error, Some(err.to_string())),
    };
    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

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
}
