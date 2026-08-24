//! Integration tests for `[crates.e2e.snippets].curated_snippets`, driven through the real
//! `generate_snippet_report_with_extensions` pipeline.
//!
//! Split out of [`super::coverage`] to keep that file under the repo's 1,000-line cap; see
//! [`super`] for the shared fixture helpers this file reuses via `use super::*;`. The
//! unit-level half of this declaration (`coverage::resolve_curated_snippet_paths` matching
//! logic, in isolation) lives in `src/e2e/snippets/coverage.rs`'s own `curated_snippet_tests`
//! module; this file's job is proving the declaration actually reaches a real
//! [`crate::e2e::snippets::SnippetGenerationReport`] end to end.

use super::*;

/// Drives the real generation pipeline end to end with `curated_snippets` configured: a
/// hand-authored file on disk that matches the glob must be resolved into
/// `report.curated_paths` and must never appear in `coverage.missing` -- the coverage-side
/// half of the gap this declaration exists to close (a curated file has no fixture behind
/// it at all, so it was previously invisible to coverage rather than reported).
#[test]
fn curated_snippets_are_resolved_into_the_report_and_never_become_missing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _cwd = crate::test_support::CwdGuard::enter(directory.path());
    std::fs::create_dir_all("docs/snippets/docker").expect("curated directory");
    std::fs::write("docs/snippets/docker/quick-start.md", "curated by hand").expect("curated file");

    let fixture = documented_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "built_in_would_fail".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        curated_snippets: vec!["docker/*.md".to_string()],
        ..SnippetConfig::default()
    };
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
        body: "extension_call()",
    })];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &extensions)
            .expect("curated declaration coexists with generated coverage");

    assert_eq!(
        report.curated_paths,
        vec![std::path::PathBuf::from("docker/quick-start.md")]
    );
    assert_eq!(report.coverage.generated.len(), 1);
    assert!(
        report.coverage.missing.is_empty(),
        "a curated file must never make an unrelated fixture cell missing: {:?}",
        report.coverage.missing
    );
}

/// The anti-vacuity requirement driven through the real pipeline rather than the unit-level
/// helper alone: a `curated_snippets` glob that matches no file must fail generation outright
/// rather than silently completing with nothing curated. A glob typo that quietly marked
/// nothing as curated would recreate the exact "coverage reports curated files as missing"
/// bug class this declaration exists to close.
#[test]
fn a_curated_glob_matching_zero_files_fails_the_real_generation_run() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _cwd = crate::test_support::CwdGuard::enter(directory.path());
    std::fs::create_dir_all("docs/snippets").expect("output directory");

    let fixture = documented_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "built_in_would_fail".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        curated_snippets: vec!["docker/*.md".to_string()],
        ..SnippetConfig::default()
    };
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
        body: "extension_call()",
    })];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let error =
        generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &extensions)
            .expect_err("a curated glob matching zero files must fail the real run");

    assert!(error.to_string().contains("matches no file"), "{error}");
}
