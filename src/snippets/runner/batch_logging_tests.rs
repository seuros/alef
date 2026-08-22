//! `run_validation` must log a matching Starting/Finished pair for whichever codepath a
//! validator actually took -- batched or the per-snippet fallback -- never a silent one.

use super::*;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;
use tracing_test::traced_test;

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
