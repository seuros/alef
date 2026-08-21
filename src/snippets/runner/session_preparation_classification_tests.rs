//! Alef defect #142: a snippet session's `before` hook builds this language's artifacts
//! (`cargo build --release -p <crate>-jni`, `pnpm run build:all`, ...) before any of its
//! snippets can validate. When that hook outlives `timeout_secs` -- readily hit on a loaded
//! machine, or right after `alef all --clean` wiped the artifacts it is meant to rebuild -- the
//! failure used to collapse into the same bare `SnippetStatus::Error` as a genuinely broken
//! snippet or a misconfigured session, with a message that was just the raw timeout text. That
//! makes three fundamentally different situations read identically in the report:
//!
//!   (a) the snippet itself is wrong                    -> a real validation failure
//!   (b) the toolchain is missing                       -> a clear skip
//!   (c) the artifact was never built / `--clean` removed it -> an ordering problem
//!
//! These tests pin all three down as distinguishable outcomes, through both runner dispatch
//! paths (`fail_fast_results` and the batched/parallel path via `batch::group_batchable_snippets`)
//! -- the two components that independently classified a session preparation failure before this
//! fix, and the split this defect turned out to be.

use super::*;
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;

/// A validator that is never actually reached in these tests -- every scenario here resolves
/// before `validate_one` would call it -- but the registry needs *something* registered for
/// `TypeScript` so a missing-validator branch never masquerades as the behavior under test.
struct UnreachableValidator;

impl SnippetValidator for UnreachableValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::TypeScript
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
        panic!("this validator must never be invoked: session preparation should short-circuit first");
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

/// A validator whose toolchain is simply not installed in this environment -- the ordinary
/// `(b)` case, distinct from an unbuilt artifact.
struct MissingToolchainValidator;

impl SnippetValidator for MissingToolchainValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::TypeScript
    }

    fn is_available(&self) -> bool {
        false
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        panic!("an unavailable toolchain must never be invoked");
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

/// A validator whose toolchain runs and genuinely rejects the snippet -- the `(a)` case, which
/// must stay a real failure rather than being pulled into the ordering bucket.
struct GenuinelyBrokenValidator;

impl SnippetValidator for GenuinelyBrokenValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::TypeScript
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
        Ok((SnippetStatus::Fail, Some("expected `;`, found end of file".to_string())))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

fn typescript_snippet() -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::TypeScript,
        title: None,
        code: "const value: number = 1;".into(),
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

fn timing_out_session(working_directory: &std::path::Path) -> HashMap<String, SessionSpec> {
    HashMap::from([(
        "typescript".to_string(),
        SessionSpec {
            language: crate::snippets::types::Language::TypeScript,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            before: vec![sleep_hook(2)],
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        },
    )])
}

#[cfg(unix)]
fn sleep_hook(seconds: u64) -> String {
    format!("sleep {seconds}")
}

#[cfg(windows)]
fn sleep_hook(seconds: u64) -> String {
    format!("timeout /t {seconds}")
}

/// Case (c), parallel/batched dispatch: `group_batchable_snippets` in `batch.rs` is the path a
/// non-`fail_fast` run takes. Before this fix it stamped `SnippetStatus::Error` directly from the
/// stringified session error, with no way to tell an unbuilt artifact from a broken session.
#[test]
fn session_preparation_timeout_is_an_ordering_problem_not_a_bare_error_on_the_parallel_path() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        timeout_secs: 1,
        sessions: timing_out_session(directory.path()),
        fail_fast: false,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(summary.errors, 0, "an unbuilt artifact must not count as a bare error");
    assert_eq!(summary.failed, 0, "an unbuilt artifact is not a snippet failure");
    assert_eq!(summary.unavailable, 1);
    assert_eq!(summary.unresolved_dependency, 1);
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Unavailable);
    assert!(outcome.unresolved_dependency);
    let message = outcome.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("ordering problem"),
        "message must name the ordering problem, not read as a bare timeout: {message}"
    );
}

/// Case (c), fail-fast dispatch: `validate_one`'s own `session_preparation_error` branch in
/// `runner.rs` is the *other* component that independently classified this failure. Both paths
/// must agree.
#[test]
fn session_preparation_timeout_is_an_ordering_problem_not_a_bare_error_on_the_fail_fast_path() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        timeout_secs: 1,
        sessions: timing_out_session(directory.path()),
        fail_fast: true,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(summary.errors, 0, "an unbuilt artifact must not count as a bare error");
    assert_eq!(summary.unavailable, 1);
    assert_eq!(summary.unresolved_dependency, 1);
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Unavailable);
    assert!(outcome.unresolved_dependency);
    assert!(
        outcome
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("ordering problem")
    );
}

/// Case (b): a genuinely missing toolchain is `Unavailable` too, but must never be mistaken for
/// case (c) -- there is no unbuilt artifact here, just no compiler on `PATH`.
#[test]
fn missing_toolchain_is_unavailable_but_not_flagged_as_unresolved_dependency() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(MissingToolchainValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.unavailable, 1);
    assert_eq!(
        summary.unresolved_dependency, 0,
        "a missing toolchain is not the same problem as an unbuilt artifact"
    );
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Unavailable);
    assert!(!outcome.unresolved_dependency);
}

/// Case (a): a validator that actually ran and rejected the snippet on its own merits must stay
/// a real failure, not be pulled into the ordering/unresolved-dependency bucket.
#[test]
fn a_genuinely_broken_snippet_stays_a_real_failure() {
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(GenuinelyBrokenValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.failed, 1);
    assert_eq!(summary.unresolved_dependency, 0);
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Fail);
    assert!(!outcome.unresolved_dependency);
}
