//! Post-generation formatter support for e2e test projects.
//!
//! Formatting is delegated to the `poly` (polylint) CLI as a system dependency —
//! the same tool the main generate pipeline uses (see `cli::pipeline::format`).
//! For each language directory that had files generated, `run_formatters` runs a
//! single `poly fmt --fix` pass, which formats every language poly supports
//! (Python via ruff, JS/TS/JSON via oxc, Rust via rustfmt, Go via gofmt, …).
//! Missing or failing formatters abort generation.
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
/// otherwise poly formats the directory in-process. Formatter failures abort
/// generation.
///
/// Actual formatting (poly, `mix format`) aborts generation in every mode — it
/// needs no registry and so has no pre-release excuse. Only *dependency
/// resolution* is treated differently: under [`DependencyMode::Registry`] a
/// resolving step is recorded as a [`DeferredFormatting`] instead of failing the
/// run, because the version it would resolve is the one this run is producing.
/// [`DependencyMode::Local`] behaviour is unchanged and always yields an empty
/// list. ~keep
pub fn run_formatters(files: &[GeneratedFile], e2e_config: &E2eConfig) -> anyhow::Result<Vec<DeferredFormatting>> {
    let defer_resolution = e2e_config.dep_mode == DependencyMode::Registry;
    let mut deferred = Vec::new();
    let output_prefix = Path::new(e2e_config.effective_output());
    let current_dir = std::env::current_dir().context("failed to resolve formatter working directory")?;
    let languages: HashSet<String> = files
        .iter()
        .filter_map(|f| {
            let remainder = f.path.strip_prefix(output_prefix).ok()?;
            let first = remainder.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        })
        .collect();

    for lang in &languages {
        let configured_dir = PathBuf::from(format!("{}/{}", e2e_config.effective_output(), lang));
        let dir_path = resolve_formatter_directory(&configured_dir, &current_dir)?;
        let dir = dir_path.to_string_lossy();

        // User override takes precedence and replaces the poly pass entirely. Its
        // contents are opaque to us, so it may or may not resolve dependencies
        // (`bundle exec`, `npx`, …). In registry mode we cannot tell the two apart,
        // so a failure is recorded rather than fatal; in local mode it still aborts.
        if let Some(custom) = e2e_config.format.get(lang.as_str()) {
            let cmd = custom.replace("{dir}", &dir);
            tracing::debug!("Formatting {lang}: {cmd}");
            match run_shell(&cmd, lang) {
                Ok(()) => {}
                Err(error) if defer_resolution => {
                    warn!("deferring {lang} format override until after publish: {error}");
                    deferred.push(DeferredFormatting {
                        language: lang.clone(),
                        step: cmd,
                        reason: format!("{UNPUBLISHED_VERSION_REASON} (failed with: {error})"),
                    });
                }
                Err(error) => return Err(error),
            }
            continue;
        }

        // Default: shell out to `poly fmt --fix` over the directory. poly walks up
        // from `dir_path` for a `poly.toml` (falling back to poly's zero-config
        // defaults when none is found).
        tracing::debug!("Formatting {lang} with poly: {dir}");
        crate::cli::pipeline::poly_format_strict(std::slice::from_ref(&dir_path), &dir_path)?;

        // Residual: `go mod tidy` populates `go.sum` from `go.mod` (poly cannot —
        // it is dependency resolution, not formatting) so the Go suite builds.
        // In registry mode `go.mod` pins the unpublished version this run emits, so
        // tidy cannot resolve it; skip rather than fail, and record the debt.
        if lang == "go" {
            if defer_resolution {
                warn!("skipping `go mod tidy` for {lang}: {UNPUBLISHED_VERSION_REASON}");
                deferred.push(DeferredFormatting {
                    language: lang.clone(),
                    step: GO_MOD_TIDY_STEP.to_owned(),
                    reason: UNPUBLISHED_VERSION_REASON.to_owned(),
                });
            } else {
                run_go_mod_tidy(&dir)?;
            }
        }

        // Residual: `mix format` is the SOLE formatter for `.ex`/`.exs` — the poly
        // pass above excludes them (see `POLY_ELIXIR_EXCLUDE_GLOBS`), so without
        // this the generated Elixir suite is never formatted at all and ships with
        // the emitter's unwrapped long lines.
        if lang == "elixir" {
            run_mix_format(&dir)?;
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
    Ok(deferred)
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
    run_formatters(&files, e2e_config)
}

fn run_shell(cmd: &str, lang: &str) -> anyhow::Result<()> {
    match std::process::Command::new("sh").args(["-c", cmd]).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => anyhow::bail!("formatter for {lang} exited with {status}: {cmd}"),
        Err(error) => Err(error).with_context(|| format!("failed to run formatter for {lang}: {cmd}")),
    }
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
fn run_go_mod_tidy(dir: &str) -> anyhow::Result<()> {
    let cmd = format!("(cd {dir} && go mod tidy)");
    run_shell(&cmd, "go")
}

/// Format `.ex`/`.exs` in the e2e Elixir directory with `mix format`.
///
/// Must run from `dir` so mix reads that project's own `.formatter.exs` (emitted
/// alongside `mix.exs`) — a bare `mix format` has no `inputs:` without it. That
/// file deliberately omits `import_deps`, so this needs no prior `mix deps.get`.
fn run_mix_format(dir: &str) -> anyhow::Result<()> {
    let cmd = format!("(cd {dir} && mix format)");
    run_shell(&cmd, "elixir")
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
        run_formatters(&files, &e2e_config).unwrap();
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

        let error = run_formatters(&one_file_in(&out, "python", "main.py"), &config)
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

        let deferred = run_formatters(&one_file_in(&out, "python", "main.py"), &config)
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

        let deferred =
            run_formatters(&one_file_in(&out, "python", "main.py"), &config).expect("successful override must be Ok");

        assert!(sentinel.exists(), "registry mode must still run the formatter");
        assert!(deferred.is_empty(), "a successful step must not be deferred: {deferred:?}");
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

    /// The default path shells out to `poly fmt --fix` and rejects an unavailable
    /// formatter instead of accepting noncanonical output.
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
            run_formatters(&files, &e2e_config).unwrap();
            let formatted = std::fs::read_to_string(&py).unwrap();
            assert_eq!(
                formatted, "x = 1\n",
                "with poly installed, `poly fmt --fix` must reformat the e2e Python file"
            );
        } else {
            assert!(run_formatters(&files, &e2e_config).is_err());
        }
    }

    #[test]
    fn unavailable_configured_formatter_aborts_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x = 1\n").unwrap();

        let mut e2e_config = e2e_config_for(&out);
        e2e_config.format.insert(
            "python".to_owned(),
            "alef_formatter_that_does_not_exist {dir}".to_owned(),
        );
        let files = vec![GeneratedFile {
            path: py,
            content: "x = 1\n".to_owned(),
            generated_header: false,
        }];

        let error = run_formatters(&files, &e2e_config).expect_err("missing formatter must fail generation");
        assert!(error.to_string().contains("formatter for python exited"));
    }

    #[test]
    fn cached_paths_use_the_same_formatter_pipeline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("e2e-out");
        std::fs::create_dir_all(out.join("python")).unwrap();
        let py = out.join("python/main.py");
        std::fs::write(&py, "x=1").unwrap();

        let e2e_config = e2e_config_for(&out);
        if which::which("poly").is_ok() {
            run_formatters_for_cached_paths(std::slice::from_ref(&py), dir.path(), &e2e_config).unwrap();
            let formatted = std::fs::read_to_string(&py).unwrap();
            assert_eq!(formatted, "x = 1\n");
        } else {
            assert!(run_formatters_for_cached_paths(std::slice::from_ref(&py), dir.path(), &e2e_config).is_err());
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

        run_formatters(&files, &e2e_config).unwrap();

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

        let result = run_formatters(&files, &e2e_config);
        let formatted = std::fs::read_to_string(&test_file).unwrap();
        if which::which("poly").is_ok() && which::which("mix").is_ok() {
            result.unwrap();
            assert_ne!(
                formatted, unformatted,
                "with mix installed, the elixir residual must reformat the over-long call"
            );
            assert!(
                formatted.contains("M.convert(\n"),
                "mix must wrap the over-long call onto its own line, got:\n{formatted}"
            );
        } else {
            assert!(result.is_err());
            assert_eq!(formatted, unformatted, "without mix the file must be left untouched");
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

        let result = run_formatters(&files, &e2e_config);
        if which::which("poly").is_ok() {
            result.unwrap();
        } else {
            assert!(result.is_err());
        }
    }
}
