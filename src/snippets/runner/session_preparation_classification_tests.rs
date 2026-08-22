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

/// task #130: a `tsc` toolchain that ran to completion and reported a genuine type error
/// (TS2322 "not assignable") must come back through `finalize_result` as `Fail` with the
/// compiler's own message, never `Unavailable` captioned "toolchain ran but reported a missing
/// dependency or build artifact -- run `alef build` first". That caption sent the reader to
/// rebuild toolchains for a defect no rebuild could fix, and `Unavailable` is an incomplete
/// status that fails a `--strict` run for a reason it had misnamed. This drives the real
/// `TypeScriptValidator::is_dependency_error`, not a stub, through `finalize_result` directly. ~keep
#[test]
fn finalize_result_keeps_a_real_type_error_as_fail_with_the_compiler_message() {
    let validator = crate::snippets::validators::typescript::TypeScriptValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let diagnostic = "snippet.ts(1,7): error TS2322: Type 'number' is not assignable to type 'string'.";
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some(diagnostic.to_string()),
        duration_ms: 5,
    };

    let result = finalize_result(
        &typescript_snippet(),
        &validator,
        &config,
        None,
        ValidationLevel::Compile,
        outcome,
    );

    assert_eq!(result.status, SnippetStatus::Fail, "got: {result:?}");
    assert!(
        !result.unresolved_dependency,
        "a real type error must not be flagged as a dependency gap"
    );
    assert_eq!(
        result.message.as_deref(),
        Some(diagnostic),
        "a real type error's message must stay the compiler's own text verbatim, not be recaptioned \
         as a missing dependency"
    );
    assert!(
        !result.message.as_deref().unwrap_or_default().contains("alef build"),
        "a real type error must never tell the reader to rebuild toolchains: {result:?}"
    );
}

/// The complementary case: a genuinely unresolved module (`tsc` could not locate it at all) must
/// still classify as `unresolved_dependency` -- proving the narrowed pattern set didn't
/// overcorrect into treating every `tsc` failure as a snippet defect.
#[test]
fn finalize_result_still_flags_a_real_missing_module_as_unresolved_dependency() {
    let validator = crate::snippets::validators::typescript::TypeScriptValidator;
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let outcome = ValidationOutcome {
        status: SnippetStatus::Fail,
        message: Some("snippet.ts(1,1): error TS2307: Cannot find module 'widgets'.".to_string()),
        duration_ms: 5,
    };

    let result = finalize_result(
        &typescript_snippet(),
        &validator,
        &config,
        None,
        ValidationLevel::Compile,
        outcome,
    );

    assert_eq!(result.status, SnippetStatus::Unavailable, "got: {result:?}");
    assert!(
        result.unresolved_dependency,
        "a genuinely unresolved module must still be flagged: {result:?}"
    );
    assert!(
        result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("Cannot find module 'widgets'"),
        "the original diagnostic must still be included, not replaced: {result:?}"
    );
}

/// Alef defect #127: two configured sessions target the same language (a real consumer's
/// `[docs.snippets.sessions.typescript]` + `[docs.snippets.sessions.wasm]`, both TypeScript) and
/// a hand-written snippet carries no explicit `target:` to break the tie. Before this fix, the
/// fallback was a literal `sessions.get("typescript")` lookup: whichever session happened to be
/// spelled like the bare language silently claimed every such snippet, validated it against that
/// session's toolchain, and reported a normal `Pass`/`Fail` -- with no signal anywhere that the
/// claim was an accident of naming, or that the sibling `wasm` session got no hand-written
/// coverage at all. `UnreachableValidator` proves the ambiguity is caught before any validator
/// ever runs, on both dispatch paths.
fn two_same_language_sessions(node: &std::path::Path, wasm: &std::path::Path) -> HashMap<String, SessionSpec> {
    let spec = |working_directory: &std::path::Path| SessionSpec {
        language: crate::snippets::types::Language::TypeScript,
        working_directory: working_directory.to_path_buf(),
        manifest: None,
        before: Vec::new(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    };
    HashMap::from([("typescript".to_string(), spec(node)), ("wasm".to_string(), spec(wasm))])
}

#[test]
fn an_ambiguous_session_claim_is_a_real_error_on_the_fail_fast_path() {
    let node = tempfile::tempdir().expect("node session directory");
    let wasm = tempfile::tempdir().expect("wasm session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: two_same_language_sessions(node.path(), wasm.path()),
        fail_fast: true,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(
        summary.failed, 0,
        "an ambiguous claim is a configuration gap, not a snippet defect"
    );
    assert_eq!(summary.errors, 1);
    assert!(
        summary.has_failures(),
        "an ambiguous claim must fail every run, not just --strict"
    );
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Error);
    let message = outcome.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("typescript"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("wasm"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("target:"),
        "message must tell the reader how to resolve it: {message}"
    );
}

#[test]
fn an_ambiguous_session_claim_is_a_real_error_on_the_parallel_path() {
    let node = tempfile::tempdir().expect("node session directory");
    let wasm = tempfile::tempdir().expect("wasm session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(UnreachableValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: two_same_language_sessions(node.path(), wasm.path()),
        fail_fast: false,
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    assert_eq!(summary.total, 1);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.errors, 1);
    assert!(summary.has_failures());
    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Error, "got: {outcome:?}");
    // The message must name the ambiguity itself, not just any `SnippetStatus::Error` --
    // `UnreachableValidator` never overrides `validate_in_session`, so if resolution ever slipped
    // back to picking a session by naming coincidence (the pre-fix bug), the *default*
    // `validate_in_session` would reject that session too and land on the same bare `Error`
    // status for a completely different reason. Only the message tells the two apart.
    let message = outcome.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("typescript"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("wasm"),
        "message must name every candidate session: {message}"
    );
    assert!(
        message.contains("target:"),
        "message must tell the reader how to resolve it: {message}"
    );
}

/// A single configured session still claims a target-less snippet no matter what it is named --
/// the other half of #127. Three of the four real consumer configs surveyed name their TypeScript
/// sessions `node`/`wasm`, never `typescript`; before this fix the bare-language fallback missed
/// every one of them and every hand-written snippet validated with no session at all.
///
/// `GenuinelyBrokenValidator` never overrides `validate_in_session`, so `SnippetValidator`'s
/// default implementation is what actually runs: it rejects outright when handed `Some(session)`
/// ("does not support binding-aware sessions") and only calls through to `validate` when handed
/// `None`. That default is a precise discriminator here -- before this fix the `node`-named
/// session never resolved for a target-less snippet, so validation fell through to `None` and
/// `GenuinelyBrokenValidator::validate`'s own `Fail`. After the fix, `session_for` resolves the
/// single same-language candidate regardless of its name, so the snippet reaches the validator
/// *with* a session and the default rejection fires instead. ~keep
#[test]
fn a_single_differently_named_session_still_claims_a_target_less_snippet() {
    let directory = tempfile::tempdir().expect("session directory");
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(GenuinelyBrokenValidator));
    let config = RunnerConfig {
        level: ValidationLevel::Compile,
        cache_dir: None,
        sessions: HashMap::from([(
            "node".to_string(),
            SessionSpec {
                language: crate::snippets::types::Language::TypeScript,
                working_directory: directory.path().to_path_buf(),
                manifest: None,
                before: Vec::new(),
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        )]),
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[typescript_snippet()], &registry, &config).expect("validation completes");

    let outcome = &summary.results[0];
    assert_eq!(outcome.status, SnippetStatus::Error, "got: {outcome:?}");
    assert!(
        outcome
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("binding-aware sessions"),
        "the snippet must have reached the validator carrying the `node` session, not `None`: {outcome:?}"
    );
}
