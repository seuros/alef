use crate::docs::enforce_snippet_summary;
use crate::snippets::types::{
    DowngradeReason, Language, RunSummary, Snippet, SnippetMetadata, SnippetStatus, SourceOrigin, ValidationLevel,
    ValidationResult,
};
use tracing_test::traced_test;

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
        unresolved_dependency: false,
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

/// Regression for `docs.snippets.validation_level = "run"` reported as unreachable: `alef e2e
/// generate` stamps every fixture snippet's front matter with `level: typecheck` (see
/// `e2e::snippets::render_snippet_markdown`), which caps `effective_validation_level` below any
/// stronger configured level. That is a legitimate per-snippet contract (`DowngradeReason::
/// Declared`), so it must not fail even in strict mode — but before this, it also produced no
/// trace anywhere that the configured level was not actually applied. A `Declared`-capped `Pass`
/// must warn, the same way a `capability_capped` one already does. ~keep
#[traced_test]
#[test]
fn declared_capped_results_warn_but_do_not_bail_even_in_strict_mode() {
    let summary = RunSummary::from_results(vec![ValidationResult {
        status: SnippetStatus::Pass,
        downgrade_reason: Some(DowngradeReason::Declared),
        message: Some("requested run, validated at declared level typecheck".to_string()),
        ..result("fixture_e2e_typecheck", SnippetStatus::Pass)
    }]);

    enforce_snippet_summary("fixture-crate", true, &summary).expect("a declared cap must not bail, even strict");

    assert!(
        logs_contain("front-matter `level:`"),
        "a declared-level cap must be warned about, not silently passed"
    );
}

/// Negative control: an ordinary clean run — nothing capped, nothing downgraded, a legitimately
/// configured lower level applied without any gap — must not trip the new warning path at all. ~keep
#[traced_test]
#[test]
fn an_ordinary_clean_run_emits_no_declared_capped_warning() {
    let summary = RunSummary::from_results(vec![result("fixture_ok", SnippetStatus::Pass)]);

    enforce_snippet_summary("fixture-crate", true, &summary).expect("a clean run must not bail");

    assert!(
        !logs_contain("front-matter `level:`"),
        "an ordinary clean run must not warn about a declared-level cap that never happened"
    );
}

/// Regression for task #186's precondition-gap defect: `unresolved_dependency` is set only when
/// a real toolchain ran to completion and reported a missing import/link/build target -- never a
/// defect in the generated bindings -- and neither caller of this validation path (`alef docs`,
/// `alef all`) ever runs a full per-language build in the same invocation. Strict mode must not
/// fail the run over it: doing so is indistinguishable, to an operator, from a genuine content
/// defect, which is exactly what trained operators to distrust an `alef all` failure. It must
/// still be reported, just not as a bail and not (task #542) as a `warn`: a corpus that is
/// *entirely* this expected shape is exactly the "config is correct for `alef snippets check
/// --level compile`, but not for a plain `alef all`" case task #542 exists to stop warning about
/// on every single generation run. ~keep
#[traced_test]
#[test]
fn unresolved_dependency_unavailable_does_not_bail_even_in_strict_mode() {
    let summary = RunSummary::from_results(vec![ValidationResult {
        status: SnippetStatus::Unavailable,
        unresolved_dependency: true,
        ..result("fixture_ts_import", SnippetStatus::Unavailable)
    }]);

    enforce_snippet_summary("fixture-crate", true, &summary)
        .expect("an unresolved-dependency-only unavailable result must not bail strict mode");

    assert!(
        logs_contain("unresolved dependency"),
        "a demoted unresolved dependency must still be reported"
    );
    assert!(
        !logs_contain("WARN"),
        "a corpus that is entirely the expected build-precondition gap must not warn (task #542)"
    );
}

/// The other half: a toolchain that is simply missing from `PATH` is a real environment gap
/// unrelated to any build artifact, and must still fail strict mode exactly as before -- the fix
/// above narrows the strict-unavailable bail, it does not remove it. ~keep
#[test]
fn toolchain_missing_unavailable_still_bails_in_strict_mode() {
    let summary = RunSummary::from_results(vec![result("fixture_zig_missing", SnippetStatus::Unavailable)]);

    let error = enforce_snippet_summary("fixture-crate", true, &summary)
        .expect_err("a genuinely missing toolchain must still fail strict mode");

    let message = error.to_string();
    assert!(message.contains("missing toolchain"), "got: {message}");
    assert!(
        !message.contains("unresolved dependency)"),
        "a pure toolchain-missing bail must not claim any unresolved dependency: {message}"
    );
}

/// Regression for task #488: a corpus that is entirely `unresolved_dependency`-unavailable must
/// still report that nothing reached the requested level, even though (per task #186, just
/// above) it must not bail -- `alef docs`/`alef all` cannot guarantee a fresh build ran in the
/// same invocation, so this is an expected shape for this pipeline, not a defect, but it must
/// never be silent either. Task #542 narrows *how* loudly: since this exact shape is expected on
/// every `alef all`/`alef docs` run configured for compile-level checking, it is reported at
/// `info`, not `warn` -- see `unresolved_dependency_unavailable_does_not_bail_even_in_strict_mode`
/// just above for the sibling report this pairs with. ~keep
#[traced_test]
#[test]
fn checked_nothing_reports_without_bailing_even_in_strict_mode() {
    let summary = RunSummary::from_results(vec![ValidationResult {
        status: SnippetStatus::Unavailable,
        unresolved_dependency: true,
        ..result("fixture_ts_import", SnippetStatus::Unavailable)
    }]);

    enforce_snippet_summary("fixture-crate", true, &summary).expect("must not bail, per task #186");

    assert!(
        logs_contain("NOT ONE reached the requested level"),
        "a corpus that checked nothing must report it regardless of the bail decision"
    );
    assert!(
        !logs_contain("WARN"),
        "a corpus that is entirely the expected build-precondition gap must not warn (task #542)"
    );
}

/// Negative control: a healthy run with at least one fully-verified result must never emit the
/// "checked nothing" warning, strict or not.
#[traced_test]
#[test]
fn a_healthy_run_never_warns_that_it_checked_nothing() {
    let summary = RunSummary::from_results(vec![result("fixture_ok", SnippetStatus::Pass)]);

    enforce_snippet_summary("fixture-crate", true, &summary).expect("a clean run must not bail");

    assert!(
        !logs_contain("NOT ONE reached the requested level"),
        "a run with a real pass must not claim it checked nothing"
    );
}

/// A mix of both causes must still bail on the toolchain-missing half -- the strict gate must
/// not be silenced just because some of the batch was a build-artifact gap instead. ~keep
#[test]
fn a_mix_of_both_causes_still_bails_on_the_toolchain_missing_half() {
    let summary = RunSummary::from_results(vec![
        ValidationResult {
            status: SnippetStatus::Unavailable,
            unresolved_dependency: true,
            ..result("fixture_ts_import", SnippetStatus::Unavailable)
        },
        result("fixture_zig_missing", SnippetStatus::Unavailable),
    ]);

    let error = enforce_snippet_summary("fixture-crate", true, &summary)
        .expect_err("a genuine toolchain gap in the same batch must still fail strict mode");

    assert!(error.to_string().contains("1 unavailable"), "got: {error}");
}
