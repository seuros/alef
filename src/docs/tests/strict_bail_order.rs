use crate::docs::enforce_snippet_summary;
use crate::snippets::types::{
    Language, RunSummary, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel, ValidationResult,
};

fn result(id: &str, status: SnippetStatus) -> ValidationResult {
    ValidationResult {
        snippet: Snippet {
            id: Some(id.to_string()),
            path: "docs/example.md".into(),
            language: Language::C,
            title: None,
            code: String::new(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "docs/example.md".into(),
                line: 3,
                block_index: 0,
            },
        },
        status,
        level: ValidationLevel::Syntax,
        requested_level: ValidationLevel::TypeCheck,
        effective_level: ValidationLevel::Syntax,
        message: None,
        duration_ms: 0,
        capability_capped: false,
        downgrade_reason: None,
    }
}

/// A run that both failed outright and downgraded something must report the failure, not the
/// downgrade. Before `enforce_snippet_summary` was factored out of `validate_snippets`, the
/// strict downgraded check ran (and bailed) before the failure check further down was ever
/// reached, so a consumer investigating "N downgraded" never learned the run had actually failed. ~keep
#[test]
fn hard_failures_bail_before_a_strict_downgrade() {
    let summary = RunSummary::from_results(vec![
        result("fixture_downgraded", SnippetStatus::Downgraded),
        result("fixture_failed", SnippetStatus::Fail),
    ]);

    let error = enforce_snippet_summary("fixture-crate", true, &summary).expect_err("must fail");

    let message = error.to_string();
    assert!(message.contains("1 failed"), "got: {message}");
    assert!(
        !message.contains("downgraded"),
        "the failure bail must return before the downgraded check ever runs, got: {message}"
    );
}

/// A session/preparation error surfaces as `SnippetStatus::Error`, and must bail the same way a
/// `Fail` does — ahead of a strict downgrade — not get silently absorbed into "just a downgrade".
#[test]
fn session_errors_bail_before_a_strict_downgrade() {
    let summary = RunSummary::from_results(vec![
        result("fixture_downgraded", SnippetStatus::Downgraded),
        result("fixture_session_error", SnippetStatus::Error),
    ]);

    let error = enforce_snippet_summary("fixture-crate", true, &summary).expect_err("must fail");

    let message = error.to_string();
    assert!(message.contains("1 errors"), "got: {message}");
    assert!(!message.contains("downgraded"), "got: {message}");
}

/// With no hard failures, a strict downgrade must still bail — the reorder must not accidentally
/// swallow the downgraded check entirely. ~keep
#[test]
fn a_strict_downgrade_still_bails_when_nothing_failed() {
    let summary = RunSummary::from_results(vec![result("fixture_downgraded", SnippetStatus::Downgraded)]);

    let error = enforce_snippet_summary("fixture-crate", true, &summary).expect_err("must fail");

    assert!(error.to_string().contains("1 validation(s) downgraded"));
}

/// A downgrade is not an error outside strict mode, failures or not.
#[test]
fn non_strict_mode_never_bails_on_a_downgrade() {
    let summary = RunSummary::from_results(vec![result("fixture_downgraded", SnippetStatus::Downgraded)]);

    enforce_snippet_summary("fixture-crate", false, &summary).expect("non-strict must not bail");
}
