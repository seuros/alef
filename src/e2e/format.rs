//! Post-generation formatter support for e2e test projects.
//!
//! Formatting is delegated to the `poly` (polylint) CLI as a system dependency —
//! the same tool the main generate pipeline uses (see `cli::pipeline::format`).
//! For each language directory that had files generated, `run_formatters` runs a
//! single `poly fmt --fix` pass, which formats every language poly supports
//! (Python via ruff, JS/TS/JSON via oxc, Rust via rustfmt, Go via gofmt, …).
//! Languages are formatted in sorted order, and every language is attempted
//! before failures are reported, so which languages get formatted never depends
//! on iteration order or on whether an earlier one failed. A formatter that RUNS
//! and rejects the code still fails generation, naming every language that failed;
//! a formatter whose executable is absent is recorded and survived instead, unless
//! `--strict` asks for the old behaviour (see [`ShellFailure`]).
//!
//! Two escape hatches remain:
//! * a per-language `E2eConfig.format` override (`sh -c`, with `{dir}` expanded)
//!   replaces the poly pass for that language;
//! * a residual `go mod tidy` runs for Go directories — it is not formatting but
//!   is required to populate `go.sum` from `go.mod` so the e2e Go suite builds.
//!
//! That second escape hatch is why this stage distinguishes *formatting* from
//! *dependency resolution*. Under [`DependencyMode::Registry`] the generated
//! manifests pin the very version the current run produces, so any step that
//! resolves them against a registry cannot succeed until that version is
//! published — see [`DeferredFormatting`]. ~keep

use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::DependencyMode;
use crate::e2e::config::E2eConfig;
use anyhow::Context as _;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

/// A dependency-resolving step that could not run because the version the
/// generated manifests pin is not published yet.
///
/// Registry mode exists to exercise *published* artifacts, so during the run that
/// produces those artifacts the pinned version does not exist in any registry.
/// Resolving it is therefore a post-publish activity, and letting it abort the
/// generation that must happen *first* makes the release unreachable: publishing
/// needs a complete run, and a complete run needs the publish. Deferring breaks
/// that cycle without skipping any actual formatting. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredFormatting {
    /// e2e language directory the step belonged to.
    pub language: String,
    /// The command, or built-in step name, that was deferred.
    pub step: String,
    /// Why it could not run now.
    pub reason: String,
}

impl std::fmt::Display for DeferredFormatting {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "[{}] {} — {}", self.language, self.step, self.reason)
    }
}

/// Recorded when a resolver is skipped outright because its input cannot exist yet.
const UNPUBLISHED_VERSION_REASON: &str = "registry-mode manifests pin the version this run produces, which is not \
                                          published yet; re-run after publishing";

/// Run per-language formatters for all languages that had files generated.
///
/// E2e files are written to `{output}/{lang}/...`, so the language is the first
/// path component after the output prefix. For each language directory: a user
/// `E2eConfig.format[lang]` override runs as a shell command (`{dir}` expanded);
/// otherwise poly formats the directory in-process. Languages run in sorted
/// order and each is attempted regardless of earlier failures; the run then
/// fails with every failing language named.
///
/// Actual formatting (poly, `mix format`) aborts generation in every mode — it
/// needs no registry and so has no pre-release excuse. Only *dependency
/// resolution* is treated differently: under [`DependencyMode::Registry`] a
/// resolving step is recorded as a [`DeferredFormatting`] instead of failing the
/// run, because the version it would resolve is the one this run is producing.
/// [`DependencyMode::Local`] behaviour is unchanged and always yields an empty
/// list. ~keep
pub fn run_formatters(
    files: &[GeneratedFile],
    e2e_config: &E2eConfig,
    strict: bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    let defer_resolution = e2e_config.dep_mode == DependencyMode::Registry;
    let mut deferred = Vec::new();
    let output_prefix = Path::new(e2e_config.effective_output());
    let current_dir = std::env::current_dir().context("failed to resolve formatter working directory")?;
    // Sorted, not `HashSet` iteration order. `HashSet` is randomly seeded per process, so the
    // order languages were formatted in varied run to run; combined with the abort-on-first-
    // failure below, a single failing language left a *different* arbitrary subset of the others
    // unformatted each time, and regenerating an unchanged tree produced different bytes. ~keep
    let mut languages: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let remainder = f.path.strip_prefix(output_prefix).ok()?;
            let first = remainder.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        })
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();
    languages.sort();

    // Format every language before reporting rather than aborting on the first failure. Whether
    // one language's formatter fails must not decide whether the others run at all -- that made
    // the emitted tree depend on ordering. The run still fails, naming every failure. ~keep
    let mut failures: Vec<String> = Vec::new();
    for lang in &languages {
        if let Err(error) = format_language(lang, e2e_config, &current_dir, defer_resolution, strict, &mut deferred) {
            failures.push(format!("{lang}: {error:#}"));
        }
    }

    // poly (and user format overrides) rewrite files via atomic rename, which
    // resets Unix permissions to 0644 — clobbering the executable bit the scaffold
    // writer set on shebang scripts (e.g. `run_tests.php`). Re-assert it so shebang
    // e2e scripts stay executable after formatting. Paths are relative to the
    // process cwd (the repo root), matching where the writer/poly operate.
    for file in files {
        if file.content.starts_with("#!")
            && let Err(e) = crate::cli::pipeline::apply_shebang_chmod(&file.path, &file.content)
        {
            warn!("failed to restore exec bit on {}: {e}", file.path.display());
        }
    }
    if !failures.is_empty() {
        anyhow::bail!(
            "formatting failed for {} of {} language(s): {}",
            failures.len(),
            languages.len(),
            failures.join("; ")
        );
    }
    Ok(deferred)
}

/// Run the configured formatter for one generated language directory.
///
/// Split out of [`run_formatters`] so a failure can be collected per language instead of
/// aborting the whole pass -- see the ordering note there. ~keep
fn format_language(
    lang: &str,
    e2e_config: &E2eConfig,
    current_dir: &Path,
    defer_resolution: bool,
    strict: bool,
    deferred: &mut Vec<DeferredFormatting>,
) -> anyhow::Result<()> {
    let configured_dir = PathBuf::from(format!("{}/{}", e2e_config.effective_output(), lang));
    let dir_path = resolve_formatter_directory(&configured_dir, current_dir)?;
    let dir = dir_path.to_string_lossy();

    // User override takes precedence and replaces the poly pass entirely. Its
    // contents are opaque to us, so registry-mode failures are deferred.
    if let Some(custom) = e2e_config.format.get(lang) {
        let cmd = custom.replace("{dir}", &dir);
        tracing::debug!("Formatting {lang}: {cmd}");
        return match run_shell(&cmd, lang) {
            Ok(()) => Ok(()),
            // A missing executable is an environment gap in every mode, so it is
            // resolved first — deferring it as an unpublished-version problem would
            // record a reason that is simply untrue. ~keep
            Err(failure) if failure.executable_missing => resolve_shell_failure(failure, lang, &cmd, strict, deferred),
            Err(failure) if defer_resolution => {
                let error = failure.error;
                warn!("deferring {lang} format override until after publish: {error}");
                deferred.push(DeferredFormatting {
                    language: lang.to_owned(),
                    step: cmd,
                    reason: format!("{UNPUBLISHED_VERSION_REASON} (failed with: {error})"),
                });
                Ok(())
            }
            Err(failure) => Err(failure.error),
        };
    }

    // Default: shell out to `poly fmt --fix` over the directory. poly walks up
    // from `dir_path` for a `poly.toml` (falling back to poly's zero-config
    // defaults when none is found).
    //
    // A missing `poly` executable is the same "environment gap" the override branch
    // above already treats leniently under non-`--strict` mode -- checked explicitly
    // here (rather than routed through `poly_format_strict`'s own bail, which cannot
    // tell a missing executable from poly running and rejecting the code) so this
    // branch honors `strict` exactly the way the override branch and this module's own
    // doc comment promise, instead of always failing hard regardless of `strict`. ~keep
    tracing::debug!("Formatting {lang} with poly: {dir}");
    if !crate::cli::pipeline::is_tool_available("poly") {
        if strict {
            anyhow::bail!("poly not found on PATH; generated output cannot be formatted");
        }
        warn!("{lang}: poly fmt skipped — executable not found; continuing so the run reaches finalisation");
        deferred.push(DeferredFormatting {
            language: lang.to_owned(),
            step: "poly fmt --fix".to_owned(),
            reason: MISSING_TOOLCHAIN_REASON.to_owned(),
        });
    } else {
        crate::cli::pipeline::poly_format_strict(std::slice::from_ref(&dir_path), &dir_path)?;
    }

    // Residual: `go mod tidy` populates `go.sum` from `go.mod` (poly cannot —
    // it is dependency resolution, not formatting) so the Go suite builds.
    if lang == "go" {
        if defer_resolution {
            warn!("skipping `go mod tidy` for {lang}: {UNPUBLISHED_VERSION_REASON}");
            deferred.push(DeferredFormatting {
                language: lang.to_owned(),
                step: GO_MOD_TIDY_STEP.to_owned(),
                reason: UNPUBLISHED_VERSION_REASON.to_owned(),
            });
        } else {
            run_go_mod_tidy(&dir, lang, strict, deferred)?;
        }
    }

    // Residual: `mix format` is the SOLE formatter for `.ex`/`.exs` — the poly
    // pass above excludes them (see `POLY_ELIXIR_EXCLUDE_GLOBS`), so without
    // this the generated Elixir suite is never formatted at all and ships with
    // the emitter's unwrapped long lines.
    if lang == "elixir" {
        run_mix_format(&dir, lang, strict, deferred)?;
    }
    Ok(())
}

fn resolve_formatter_directory(path: &Path, current_dir: &Path) -> anyhow::Result<PathBuf> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    absolute_path
        .canonicalize()
        .with_context(|| format!("generated formatter path does not exist: {}", absolute_path.display()))
}

/// Format files restored from the generation-stage cache.
///
/// Cached paths are absolute while e2e generation records paths relative to the
/// consumer root. Rebuilding the lightweight file descriptors keeps cache hits on
/// the same formatter path as fresh generation, including custom format commands
/// and executable-bit restoration. ~keep
pub fn run_formatters_for_cached_paths(
    paths: &[PathBuf],
    base_dir: &Path,
    e2e_config: &E2eConfig,
    strict: bool,
) -> anyhow::Result<Vec<DeferredFormatting>> {
    let output_is_absolute = Path::new(e2e_config.effective_output()).is_absolute();
    let files: Vec<GeneratedFile> = paths
        .iter()
        .filter_map(|path| {
            let formatter_path = if output_is_absolute {
                path.clone()
            } else {
                path.strip_prefix(base_dir).ok()?.to_path_buf()
            };
            let content = std::fs::read_to_string(path).unwrap_or_default();
            Some(GeneratedFile {
                path: formatter_path,
                content,
                generated_header: true,
            })
        })
        .collect();
    run_formatters(&files, e2e_config, strict)
}

/// The status a POSIX shell exits with when the command it was asked to run does not
/// exist. ~keep
const SHELL_COMMAND_NOT_FOUND: i32 = 127;

/// A shell-invoked formatter that did not succeed, and whether the executable was
/// even there.
///
/// `sh -c` starts fine whether or not the formatter exists, so both cases arrive as a
/// non-zero exit and the `Err` arm of `Command::status()` is never reached. Only a
/// formatter that RAN is a verdict on the generated code; a missing one is an
/// environment gap. Conflating them made the pipeline fresh-clone hostile — `vendor/`
/// is gitignored in consumers, so a fresh clone has no php-cs-fixer and the run died
/// before `finalize_hashes`, leaving a correctly generated but entirely unstamped tree:
/// invisible to `alef verify`, failing the consumer's poly gate, and
/// byte-indistinguishable from a marker-stripping bug. ~keep
struct ShellFailure {
    executable_missing: bool,
    error: anyhow::Error,
}

fn run_shell(cmd: &str, lang: &str) -> Result<(), ShellFailure> {
    match std::process::Command::new("sh").args(["-c", cmd]).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(ShellFailure {
            executable_missing: status.code() == Some(SHELL_COMMAND_NOT_FOUND),
            error: anyhow::anyhow!("formatter for {lang} exited with {status}: {cmd}"),
        }),
        Err(error) => Err(ShellFailure {
            executable_missing: error.kind() == std::io::ErrorKind::NotFound,
            error: anyhow::Error::new(error).context(format!("failed to run formatter for {lang}: {cmd}")),
        }),
    }
}

/// Reason recorded when a formatter's executable is not installed on this machine.
const MISSING_TOOLCHAIN_REASON: &str = "the formatter's executable is not installed on this machine; generation \
                                        continued so the run still reaches finalisation. Install the toolchain, or \
                                        re-run with --strict to make this fatal";

/// Decide what an unsuccessful shell formatter means for the run.
///
/// A formatter that ran and rejected the code always fails the run — that is what
/// actually gates correctness, and it is unchanged. A formatter that is simply absent
/// is recorded and survived, unless `strict` asks for the old behaviour back.
///
/// Recording rather than skipping quietly is the whole point: an absent formatter that
/// nothing reports is the same shape as a check that passes while examining nothing. ~keep
fn resolve_shell_failure(
    failure: ShellFailure,
    lang: &str,
    step: &str,
    strict: bool,
    deferred: &mut Vec<DeferredFormatting>,
) -> anyhow::Result<()> {
    if !failure.executable_missing || strict {
        return Err(failure.error);
    }
    warn!("{lang}: `{step}` skipped — executable not found; continuing so the run reaches finalisation");
    deferred.push(DeferredFormatting {
        language: lang.to_owned(),
        step: step.to_owned(),
        reason: MISSING_TOOLCHAIN_REASON.to_owned(),
    });
    Ok(())
}

/// Step name recorded when `go mod tidy` is deferred.
const GO_MOD_TIDY_STEP: &str = "go mod tidy";

/// Log any deferred resolution steps.
///
/// For standalone stage commands, which have no later pipeline phase to report
/// from the way `alef all` does. ~keep
pub fn warn_deferred(deferred: &[DeferredFormatting]) {
    for entry in deferred {
        warn!("deferred until the pinned version is published: {entry}");
    }
}

/// Populate `go.sum` from `go.mod` in the e2e Go directory.
fn run_go_mod_tidy(dir: &str, lang: &str, strict: bool, deferred: &mut Vec<DeferredFormatting>) -> anyhow::Result<()> {
    let cmd = format!("(cd {dir} && go mod tidy)");
    match run_shell(&cmd, "go") {
        Ok(()) => Ok(()),
        Err(failure) => resolve_shell_failure(failure, lang, GO_MOD_TIDY_STEP, strict, deferred),
    }
}

/// Format `.ex`/`.exs` in the e2e Elixir directory with `mix format`.
///
/// Must run from `dir` so mix reads that project's own `.formatter.exs` (emitted
/// alongside `mix.exs`) — a bare `mix format` has no `inputs:` without it. That
/// file deliberately omits `import_deps`, so this needs no prior `mix deps.get`.
fn run_mix_format(dir: &str, lang: &str, strict: bool, deferred: &mut Vec<DeferredFormatting>) -> anyhow::Result<()> {
    let cmd = format!("(cd {dir} && mix format)");
    match run_shell(&cmd, "elixir") {
        Ok(()) => Ok(()),
        Err(failure) => resolve_shell_failure(failure, lang, "mix format", strict, deferred),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `E2eConfig` whose output directory is `out`, defaults otherwise.
    fn e2e_config_for(out: &Path) -> E2eConfig {
        E2eConfig {
            output: out.to_string_lossy().into_owned(),
            ..E2eConfig::default()
        }
    }

    /// Assert `deferred` records exactly the absence of `step` for `language`.
    ///
    /// A missing formatter executable is not an error under non-`--strict` mode -- it is a
    /// recorded deferral (see `resolve_shell_failure`). Asserting on the record rather than
    /// merely on `Ok` is the point: a run that skipped silently and a run that formatted
    /// everything both look like `Ok`, and only one of them is correct. ~keep
    fn assert_deferred(deferred: &[DeferredFormatting], language: &str, step: &str) {
        assert!(
            deferred.iter().any(|entry| entry.language == language
                && entry.step == step
                && entry.reason == MISSING_TOOLCHAIN_REASON),
            "expected a deferred `{step}` for {language}, got: {deferred:?}"
        );
    }

    #[test]
    fn formatter_directory_resolves_relative_targets_against_launch_directory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let output = directory.path().join("e2e").join("python");
        std::fs::create_dir_all(&output).expect("create formatter target");

        let resolved = resolve_formatter_directory(Path::new("e2e/python"), directory.path()).expect("resolve path");

        assert!(resolved.is_absolute());
        assert_eq!(resolved, output.canonicalize().expect("canonical output"));
    }

    #[test]
    fn formatter_directory_rejects_real_missing_targets() {
        let directory = tempfile::tempdir().expect("tempdir");
        let error = resolve_formatter_directory(Path::new("e2e/missing"), directory.path())
            .expect_err("missing formatter target must fail");

        assert!(error.to_string().contains("generated formatter path does not exist"));
    }

    /// A user override in `E2eConfig.format` must replace the poly pass: the
    /// `{dir}` placeholder is expanded and the command is run verbatim.
    #[test]
    fn user_override_command_is_expanded_and_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out = base.join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let sentinel = out.join("python/was_run.txt");
        let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");

        let mut e2e_config = e2e_config_for(&out);
        e2e_config
            .format
            .insert("python".to_owned(), format!("touch {sentinel_str}"));

        let files = vec![GeneratedFile {
            path: out.join("python/main.py"),
            content: "x = 1\n".to_owned(),
            generated_header: false,
        }];

        assert!(!sentinel.exists());
        run_formatters(&files, &e2e_config, false).unwrap();
        assert!(
            sentinel.exists(),
            "user override command must run with {{dir}} expanded"
        );
    }

    /// Build a config whose output is `out` and whose format override for `lang`
    /// is `command`, in the given dependency mode.
    ///
    /// Registry mode resolves paths through `registry.output`, not `output` (see
    /// `E2eConfig::effective_output`), so both are pointed at `out` to keep the two
    /// modes comparing the same directory. ~keep
    fn config_with_override(out: &Path, lang: &str, command: &str, dep_mode: DependencyMode) -> E2eConfig {
        let mut config = e2e_config_for(out);
        config.registry.output = out.to_string_lossy().into_owned();
        config.dep_mode = dep_mode;
        config.format.insert(lang.to_owned(), command.to_owned());
        config
    }

    fn one_file_in(out: &Path, lang: &str, name: &str) -> Vec<GeneratedFile> {
        vec![GeneratedFile {
            path: out.join(lang).join(name),
            content: "x = 1\n".to_owned(),
            generated_header: false,
        }]
    }

    /// Local mode is the correctness gate and must keep aborting on any formatter
    /// failure. This is the control for the registry-mode test below: without it, a
    /// passing deferral test could just mean failures are swallowed everywhere.
    #[test]
    fn local_mode_still_aborts_when_a_format_override_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let config = config_with_override(&out, "python", "exit 3", DependencyMode::Local);

        let error = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
            .expect_err("local mode must abort on a failing formatter");

        assert!(
            error.to_string().contains("formatter for python exited"),
            "expected the formatter failure to propagate, got: {error}"
        );
    }

    /// The defect: a registry-mode resolver failure aborted the run, which took
    /// finalisation and docs down with it. It must now be reported and survived.
    #[test]
    fn registry_mode_defers_a_failing_format_override_instead_of_aborting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let config = config_with_override(&out, "python", "exit 3", DependencyMode::Registry);

        let deferred = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
            .expect("registry mode must not abort when a resolver cannot run pre-publish");

        assert_eq!(deferred.len(), 1, "expected exactly one deferred step: {deferred:?}");
        assert_eq!(deferred[0].language, "python");
        assert_eq!(deferred[0].step, "exit 3");
        assert!(
            deferred[0].reason.contains("not published yet"),
            "reason must name the unpublished pin, got: {}",
            deferred[0].reason
        );
    }

    /// Deferral is for failures only — a registry-mode override that succeeds must
    /// still run and must report nothing, so the list cannot become a dumping ground.
    #[test]
    fn registry_mode_reports_nothing_when_the_override_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let sentinel = out.join("python/ran.txt");
        let sentinel_str = sentinel.to_string_lossy().replace('\\', "/");
        let config = config_with_override(
            &out,
            "python",
            &format!("touch {sentinel_str}"),
            DependencyMode::Registry,
        );

        let deferred = run_formatters(&one_file_in(&out, "python", "main.py"), &config, false)
            .expect("successful override must be Ok");

        assert!(sentinel.exists(), "registry mode must still run the formatter");
        assert!(
            deferred.is_empty(),
            "a successful step must not be deferred: {deferred:?}"
        );
    }

    /// `go mod tidy` is dependency resolution, not formatting, and in registry mode
    /// its input pins an unpublished version. It is skipped and recorded rather than
    /// run-and-failed. Driven through the override map so the test needs no Go
    /// toolchain: the override path proves the same defer/abort split.
    #[test]
    fn deferred_entry_renders_language_step_and_reason() {
        let entry = DeferredFormatting {
            language: "go".to_owned(),
            step: GO_MOD_TIDY_STEP.to_owned(),
            reason: UNPUBLISHED_VERSION_REASON.to_owned(),
        };

        let rendered = entry.to_string();

        assert!(rendered.starts_with("[go] go mod tidy — "), "got: {rendered}");
        assert!(rendered.contains("not published yet"), "got: {rendered}");
    }

    /// The default path shells out to `poly fmt --fix`. With poly installed it must
    /// actually reformat the file; without it, non-strict mode must defer rather than
    /// abort instead of the old behaviour of aborting regardless of `strict` -- a
    /// missing default-path formatter is the same environment gap the override branch
    /// already tolerated via `resolve_shell_failure`. Branches on the runner's real
    /// `PATH` rather than forging one: mutating process-wide `PATH` is shared mutable
    /// state across every test in this binary, the same hazard class documented on
    /// `test_support::CWD_LOCK` for `set_current_dir`, and no such lock exists for env
    /// vars here. ~keep
    #[test]
    fn default_path_formats_python_with_poly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out = base.join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x=1").unwrap();

        let e2e_config = e2e_config_for(&out);

        let files = vec![GeneratedFile {
            path: out.join("python/main.py"),
            content: "x=1".to_owned(),
            generated_header: false,
        }];

        if which::which("poly").is_ok() {
            run_formatters(&files, &e2e_config, false).unwrap();
            let formatted = std::fs::read_to_string(&py).unwrap();
            assert_eq!(
                formatted, "x = 1\n",
                "with poly installed, `poly fmt --fix` must reformat the e2e Python file"
            );
        } else {
            let deferred = run_formatters(&files, &e2e_config, false)
                .expect("non-strict mode must defer a missing default-path formatter, not abort");
            assert_eq!(deferred.len(), 1, "expected exactly one deferred step: {deferred:?}");
            assert_eq!(deferred[0].language, "python");
        }
    }

    /// Config whose python formatter names an executable that cannot exist.
    fn config_with_absent_formatter(out: &Path) -> (E2eConfig, Vec<GeneratedFile>) {
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x = 1\n").unwrap();
        let mut e2e_config = e2e_config_for(out);
        e2e_config.format.insert(
            "python".to_owned(),
            "alef_formatter_that_does_not_exist {dir}".to_owned(),
        );
        let files = vec![GeneratedFile {
            path: py,
            content: "x = 1\n".to_owned(),
            generated_header: false,
        }];
        (e2e_config, files)
    }

    /// `--strict` keeps the original contract: an absent formatter is fatal. The
    /// contract is preserved rather than deleted — it is now opt-in instead of
    /// mandatory. ~keep
    #[test]
    fn unavailable_configured_formatter_aborts_generation_under_strict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        let (e2e_config, files) = config_with_absent_formatter(&out);

        let error = run_formatters(&files, &e2e_config, true).expect_err("strict must fail on a missing formatter");
        assert!(
            error.to_string().contains("formatter for python exited"),
            "got: {error}"
        );
    }

    /// THE DEFAULT PATH, and the reason this changed: `vendor/` is gitignored in
    /// consumers, so a fresh clone has no php-cs-fixer and the run died before
    /// `finalize_hashes` — leaving a correctly generated but entirely unstamped tree,
    /// byte-indistinguishable from a marker-stripping bug.
    ///
    /// Surviving is only half of it. The step must be RECORDED, naming the language and
    /// the command, or an absent formatter becomes a check that passed while doing
    /// nothing — the exact shape this whole class of defect takes. ~keep
    #[test]
    fn unavailable_configured_formatter_is_recorded_and_survived_by_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        let (e2e_config, files) = config_with_absent_formatter(&out);

        let deferred = run_formatters(&files, &e2e_config, false).expect("a missing formatter must not abort the run");

        assert_eq!(deferred.len(), 1, "the skip must be recorded, got: {deferred:?}");
        assert_eq!(deferred[0].language, "python");
        assert!(
            deferred[0].step.contains("alef_formatter_that_does_not_exist"),
            "the record must name the command that could not run, got: {}",
            deferred[0].step
        );
        assert!(
            deferred[0].reason.contains("not installed"),
            "the record must say why, got: {}",
            deferred[0].reason
        );
    }

    /// The control that keeps the default honest. A formatter that RUNS and rejects the
    /// code is a verdict on the generated output and still fails, with no `--strict`
    /// needed — otherwise the change above would have quietly disabled the thing that
    /// actually gates correctness. ~keep
    #[test]
    fn a_formatter_that_runs_and_fails_still_aborts_without_strict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x = 1\n").unwrap();
        let mut e2e_config = e2e_config_for(&out);
        e2e_config.format.insert("python".to_owned(), "exit 3".to_owned());
        let files = vec![GeneratedFile {
            path: py,
            content: "x = 1\n".to_owned(),
            generated_header: false,
        }];

        let error = run_formatters(&files, &e2e_config, false).expect_err("a formatter that ran and failed must abort");
        assert!(
            error.to_string().contains("formatter for python exited"),
            "got: {error}"
        );
    }

    #[test]
    fn cached_paths_use_the_same_formatter_pipeline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x=1").unwrap();

        let e2e_config = e2e_config_for(&out);
        let deferred = run_formatters_for_cached_paths(std::slice::from_ref(&py), dir.path(), &e2e_config, false)
            .expect("a missing poly is deferred, not fatal, under non-strict mode");
        let formatted = std::fs::read_to_string(&py).unwrap();
        if which::which("poly").is_ok() {
            assert_eq!(formatted, "x = 1\n");
            assert!(deferred.is_empty(), "poly is installed, nothing to defer: {deferred:?}");
        } else {
            assert_eq!(formatted, "x=1", "without poly the cached file must be left untouched");
            assert_deferred(&deferred, "python", "poly fmt --fix");
        }
    }

    /// poly (and user format overrides) rewrite files via atomic rename, which
    /// resets Unix permissions to 0644. run_formatters must re-assert the
    /// executable bit on shebang scripts (e.g. `run_tests.php`) afterward, so the
    /// generated suite stays runnable. Deterministic with or without poly: absent
    /// poly leaves the file 0644, present poly may clobber it — either way the
    /// post-format chmod pass restores the bit.
    #[cfg(unix)]
    #[test]
    fn run_formatters_restores_exec_bit_on_shebang_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("php")).unwrap();
        let script = out.join("php/run_tests.php");
        let content = "#!/usr/bin/env php\n<?php\n";
        std::fs::write(&script, content).unwrap();
        // Start non-executable to prove run_formatters sets the bit.
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();

        let e2e_config = e2e_config_for(&out);
        let files = vec![GeneratedFile {
            path: script.clone(),
            content: content.to_owned(),
            generated_header: false,
        }];

        run_formatters(&files, &e2e_config, false).unwrap();

        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "shebang script must be executable after run_formatters, got mode {mode:#o}"
        );
    }

    /// `.ex`/`.exs` are excluded from the poly pass, so the Elixir residual is the
    /// only thing that can format them: without it the generated suite ships with
    /// the emitter's unwrapped long lines. Uses a call well past the emitted
    /// `.formatter.exs`'s `line_length: 140` so mix is forced to wrap it — proving
    /// mix ran, not merely that the file was left alone.
    #[test]
    fn default_path_formats_elixir_with_mix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("elixir/test")).unwrap();
        std::fs::write(
            out.join("elixir/.formatter.exs"),
            "[\n  inputs: [\"{mix,.formatter}.exs\", \"{config,lib,test}/**/*.{ex,exs}\"],\n  line_length: 140\n]\n",
        )
        .unwrap();
        let long_call = format!("<blockquote><p>{}</p></blockquote>", "x".repeat(160));
        let unformatted =
            format!("defmodule T do\n  def go do\n    {{:ok, r}} = M.convert(\"{long_call}\")\n  end\nend\n");
        let test_file = out.join("elixir/test/smoke_test.exs");
        std::fs::write(&test_file, &unformatted).unwrap();

        let e2e_config = e2e_config_for(&out);
        let files = vec![GeneratedFile {
            path: test_file.clone(),
            content: unformatted.clone(),
            generated_header: false,
        }];

        let deferred =
            run_formatters(&files, &e2e_config, false).expect("absent toolchains are deferred, not fatal, here");
        let formatted = std::fs::read_to_string(&test_file).unwrap();

        // poly excludes `.ex`/`.exs`, so mix alone decides whether this file is rewritten --
        // independently of whether poly itself is installed. Each absent tool is asserted on
        // its own deferral record. ~keep
        if which::which("mix").is_ok() {
            assert_ne!(
                formatted, unformatted,
                "with mix installed, the elixir residual must reformat the over-long call"
            );
            assert!(
                formatted.contains("M.convert(\n"),
                "mix must wrap the over-long call onto its own line, got:\n{formatted}"
            );
        } else {
            assert_eq!(formatted, unformatted, "without mix the file must be left untouched");
            assert_deferred(&deferred, "elixir", "mix format");
        }
        if which::which("poly").is_err() {
            assert_deferred(&deferred, "elixir", "poly fmt --fix");
        }
    }

    /// A language poly does not know still runs cleanly (poly no-ops on unknown
    /// files); the process must not panic or abort.
    #[test]
    fn unknown_language_dir_is_best_effort() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let out = base.join("e2e-out");
        std::fs::create_dir_all(out.join("cobol")).unwrap();

        let e2e_config = e2e_config_for(&out);

        let files = vec![GeneratedFile {
            path: out.join("cobol/main.cob"),
            content: "       IDENTIFICATION DIVISION.\n".to_owned(),
            generated_header: false,
        }];

        let deferred = run_formatters(&files, &e2e_config, false)
            .expect("an unknown language is best-effort whether or not poly is installed");
        if which::which("poly").is_ok() {
            assert!(deferred.is_empty(), "poly is installed, nothing to defer: {deferred:?}");
        } else {
            assert_deferred(&deferred, "cobol", "poly fmt --fix");
        }
    }

    /// Build an output tree with one directory per language, each with a format override that
    /// appends its own name to `log`, so a completed run records the order languages ran in.
    fn config_recording_order(out: &Path, log: &Path, languages: &[&str]) -> E2eConfig {
        let mut e2e_config = e2e_config_for(out);
        let log_str = log.to_string_lossy().replace('\\', "/");
        for lang in languages {
            std::fs::create_dir_all(out.join(lang)).expect("create language dir");
            e2e_config
                .format
                .insert((*lang).to_owned(), format!("echo {lang} >> {log_str}"));
        }
        e2e_config
    }

    fn files_for(out: &Path, languages: &[&str]) -> Vec<GeneratedFile> {
        languages
            .iter()
            .map(|lang| GeneratedFile {
                path: out.join(lang).join("main.txt"),
                content: String::new(),
                generated_header: false,
            })
            .collect()
    }

    /// Languages were collected into a `HashSet`, whose iteration order is randomly seeded per
    /// instance, so two runs over an unchanged tree formatted in different orders. That is
    /// invisible on its own, but combined with abort-on-first-failure it made the emitted bytes
    /// depend on chance. Order must be stable across runs.
    #[test]
    fn languages_are_formatted_in_a_deterministic_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        let log = dir.path().join("order.log");
        let languages = ["python", "csharp", "go", "ruby", "elixir", "dart"];

        let e2e_config = config_recording_order(&out, &log, &languages);
        let files = files_for(&out, &languages);

        run_formatters(&files, &e2e_config, false).expect("first pass");
        let first = std::fs::read_to_string(&log).expect("order log");
        std::fs::remove_file(&log).expect("reset log");
        run_formatters(&files, &e2e_config, false).expect("second pass");
        let second = std::fs::read_to_string(&log).expect("order log");

        let mut expected = languages.to_vec();
        expected.sort_unstable();
        let expected = expected
            .iter()
            .map(|lang| format!("{lang}\n"))
            .collect::<Vec<_>>()
            .join("");

        assert_eq!(first, expected, "languages must be formatted in sorted order");
        assert_eq!(
            second, first,
            "two runs over an unchanged tree must format in the same order"
        );
    }

    /// One language's formatter failing must not decide whether the rest run. Aborting on the
    /// first failure left every later language unformatted, and since the order was random, a
    /// different arbitrary subset was skipped each run.
    #[test]
    fn a_failing_language_does_not_skip_the_others() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        let log = dir.path().join("order.log");
        let languages = ["python", "csharp", "go"];

        let mut e2e_config = config_recording_order(&out, &log, &languages);
        // `csharp` sorts first, so under abort-on-first-failure nothing else would run.
        e2e_config.format.insert("csharp".to_owned(), "exit 1".to_owned());
        let files = files_for(&out, &languages);

        let error = run_formatters(&files, &e2e_config, false).expect_err("a failing formatter must fail the run");

        let recorded = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            recorded.contains("go") && recorded.contains("python"),
            "languages after the failing one must still be formatted, recorded: {recorded:?}"
        );
        let message = format!("{error:#}");
        assert!(
            message.contains("csharp"),
            "the failing language must be named, got: {message}"
        );
        assert!(
            message.contains("1 of 3"),
            "the report must say how many of how many failed, got: {message}"
        );
    }

    /// Every failure is reported, not just the first, so one run surfaces the whole picture.
    #[test]
    fn every_failing_language_is_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        let log = dir.path().join("order.log");
        let languages = ["python", "csharp", "go"];

        let mut e2e_config = config_recording_order(&out, &log, &languages);
        e2e_config.format.insert("csharp".to_owned(), "exit 1".to_owned());
        e2e_config.format.insert("go".to_owned(), "exit 1".to_owned());
        let files = files_for(&out, &languages);

        let error = run_formatters(&files, &e2e_config, false).expect_err("failing formatters must fail the run");

        let message = format!("{error:#}");
        assert!(message.contains("csharp"), "got: {message}");
        assert!(message.contains("go"), "got: {message}");
        assert!(message.contains("2 of 3"), "got: {message}");
    }
}
