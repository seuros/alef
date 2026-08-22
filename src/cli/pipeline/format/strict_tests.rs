//! The `--strict` contract for the pass that formats `packages/<lang>` -- the shipped bindings.
//!
//! `alef all --strict` documents itself as failing "when a configured formatter's executable is
//! not installed", and for a long time only `e2e::format` honoured that. This module pins the
//! other half: the same [`DeferredFormatting`] record type, the same default (warn + record, so a
//! fresh clone missing `poly`/`rustfmt`/`cargo-sort`/`mix` still completes and stamps its tree),
//! and the same escalation under `strict`.
//!
//! Every test drives executable presence through
//! [`format_generated_reporting_with`]'s injected probe rather than the host's PATH: a test whose
//! outcome depends on whether the machine running it happens to have `poly` installed proves
//! nothing on the machine where it passes. ~keep

use super::*;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    cfg.resolve().unwrap().remove(0)
}

/// A tree with a generated Python package and a root `Cargo.toml`, so both the whole-tree
/// formatters (`poly fmt`, `cargo fmt`, the workspace `cargo sort`) and the per-language pass
/// have something real to act on.
fn write_tree(base: &Path) {
    let package = base.join("packages/python");
    std::fs::create_dir_all(&package).expect("package dir");
    std::fs::write(package.join("sample.py"), "x=1").expect("generated file");
    std::fs::write(base.join("Cargo.toml"), "[workspace]\nmembers = []\n").expect("root manifest");
}

fn generated_files() -> Vec<(Language, Vec<GeneratedFile>)> {
    vec![(
        Language::Python,
        vec![GeneratedFile {
            path: PathBuf::from("packages/python/sample.py"),
            content: "x=1".to_owned(),
            generated_header: false,
        }],
    )]
}

/// Nothing installed, no `--strict`: the run must survive, and every step that did not run must
/// come back to the caller as a record. Surviving is the point -- these are host toolchains a
/// contributor may legitimately lack, and a default-fatal pass breaks every fresh clone. Being
/// *recorded* is equally the point: a skip nothing reports is the same shape as a check that
/// passes while examining nothing, which is exactly what the `warn!`-only version was. ~keep
#[test]
fn an_uninstalled_package_formatter_is_recorded_and_survived_by_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());

    let skipped =
        format_generated_reporting_with(&generated_files(), &make_config(), dir.path(), None, false, &|_tool| {
            false
        })
        .expect("a missing formatter must not fail the run without --strict");

    let steps: Vec<&str> = skipped.iter().map(|entry| entry.step.as_str()).collect();
    assert!(
        steps.iter().any(|step| step.starts_with(POLY_FMT_STEP)),
        "the skipped `poly fmt --fix` pass must reach the caller, not just a warning: {steps:?}"
    );
    assert!(
        steps.iter().any(|step| step.starts_with(CARGO_FMT_STEP)),
        "the skipped workspace `cargo fmt` must reach the caller: {steps:?}"
    );
    assert!(
        steps.iter().any(|step| step.starts_with(CARGO_SORT_STEP)),
        "the skipped workspace `cargo sort` must reach the caller: {steps:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("packages/python/sample.py")).unwrap(),
        "x=1",
        "with no formatter installed the generated file must be left exactly as emitted"
    );
}

/// The same run under `--strict` must fail, and the error must name what to install.
#[test]
fn an_uninstalled_package_formatter_aborts_the_run_under_strict() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());

    let error = format_generated_reporting_with(&generated_files(), &make_config(), dir.path(), None, true, &|_tool| {
        false
    })
    .expect_err("--strict must fail the run when a configured formatter is not installed");

    let message = format!("{error:#}");
    assert!(
        message.contains("--strict"),
        "the failure must say which flag made it fatal: {message}"
    );
    assert!(
        message.contains(POLY_FMT_STEP),
        "the failure must name the step that could not run: {message}"
    );
}

/// `--strict` is about executables that are absent, not about formatting that was not needed.
/// With every tool present there is nothing to escalate, so strict and lenient runs must agree.
#[test]
fn strict_is_inert_when_every_formatter_is_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());
    let only: HashSet<Language> = [Language::Python].into_iter().collect();

    // A partial regen over a package dir that does not exist yet formats nothing and can
    // therefore skip nothing -- the case that must stay silent under `--strict`.
    let skipped = format_generated_reporting_with(
        &generated_files(),
        &make_config(),
        &dir.path().join("absent"),
        Some(&only),
        true,
        &|_tool| true,
    )
    .expect("nothing skipped means nothing to escalate");
    assert!(skipped.is_empty(), "no step was skipped, so nothing may be recorded");
}

/// A per-language residual (`mix format` for Elixir, whose `.ex`/`.exs` output poly is
/// deliberately excluded from) must record under its own language, not the whole-tree scope --
/// the operator action differs, and Elixir is the one language with no formatted fallback at all.
#[test]
fn a_missing_language_residual_records_under_its_own_language() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let elixir = base.join("packages/elixir");
    std::fs::create_dir_all(&elixir).expect("elixir package dir");
    std::fs::write(elixir.join("mix.exs"), "defmodule Sample.MixProject do\nend\n").expect("mix.exs");

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "sample-model"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid config");
    let config = cfg.resolve().unwrap().remove(0);
    let files: Vec<(Language, Vec<GeneratedFile>)> = vec![(Language::Elixir, vec![])];
    let only: HashSet<Language> = [Language::Elixir].into_iter().collect();

    let skipped = format_generated_reporting_with(&files, &config, base, Some(&only), false, &|tool| tool != "mix")
        .expect("a missing residual must not fail the run without --strict");

    let mix = skipped
        .iter()
        .find(|entry| entry.step.starts_with("mix format"))
        .unwrap_or_else(|| panic!("`mix format` must be recorded when mix is absent: {skipped:?}"));
    assert_eq!(
        mix.language, "elixir",
        "a per-language residual must record under its language, not the whole-tree scope"
    );
}

/// [`DeferredFormatting::is_missing_toolchain`] classifies by exact string equality against a
/// constant that is private to `e2e::format`, so this module spells the reason out a second time.
/// If the two spellings ever drift, every package-formatter skip silently reclassifies as a
/// "waiting for a publish" entry -- filed under benign release-cycle noise, read by nobody, and
/// no longer escalated by `--strict`. This is the test that makes that drift impossible to land
/// quietly. ~keep
#[test]
fn a_recorded_skip_is_classified_as_a_missing_toolchain() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path());

    let skipped =
        format_generated_reporting_with(&generated_files(), &make_config(), dir.path(), None, false, &|_tool| {
            false
        })
        .expect("lenient run");

    assert!(
        !skipped.is_empty(),
        "the probe reports every tool absent, so steps were skipped"
    );
    for entry in &skipped {
        assert!(
            entry.is_missing_toolchain(),
            "every recorded package-formatter skip must classify as a missing toolchain, or the \
             shared reporter files it under the wrong heading and --strict stops escalating it: \
             {entry:?}"
        );
        assert!(
            entry.step.contains("missing: "),
            "the record must name the executable to install: {entry:?}"
        );
    }
}
