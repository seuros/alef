use crate::snippets::cache::ValidationCache;
use crate::snippets::error::Result;
use crate::snippets::types::{
    RunSummary, SideEffectClass, Snippet, SnippetAnnotationKind, SnippetStatus, ValidationLevel, ValidationResult,
};
use crate::snippets::validators::ValidatorRegistry;
use rayon::prelude::*;
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
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.parallelism)
        .build()
        .map_err(|err| crate::snippets::error::Error::Other(format!("failed to build thread pool: {err}")))?;

    let fail_fast = config.fail_fast;
    let results: Vec<ValidationResult> = pool.install(|| {
        if fail_fast {
            let mut results = Vec::with_capacity(snippets.len());
            for snippet in snippets {
                let result = validate_one(snippet, registry, config);
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
                .map(|snippet| validate_one(snippet, registry, config))
                .collect()
        }
    });

    Ok(RunSummary::from_results(results))
}

fn validate_one(snippet: &Snippet, registry: &ValidatorRegistry, config: &RunnerConfig) -> ValidationResult {
    if let Some(result) = cached_result(snippet, config) {
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
    let (mut status, message) = match validator.validate(snippet, effective_level, config.timeout_secs) {
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
        && let Err(error) = cache.store(snippet, config.level, &result)
    {
        tracing::warn!("writing snippet validation cache: {error}");
    }
    result
}

fn cached_result(snippet: &Snippet, config: &RunnerConfig) -> Option<ValidationResult> {
    if !config.changed_only {
        return None;
    }
    let cache = config.cache_dir.clone().map(ValidationCache::new)?;
    let mut result = cache.load(snippet, config.level)?;
    result.snippet = snippet.clone();
    result.duration_ms = 0;
    result.message = result.message.or_else(|| Some("cached".to_string()));
    Some(result)
}

fn side_effect_rejection(snippet: &Snippet, config: &RunnerConfig) -> Option<String> {
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
