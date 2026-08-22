use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command_streamed_with_env};
use crate::cli::registry;
use crate::core::config::output::{StringOrVec, TestConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::publish::ffi_stage;
use crate::publish::platform::RustTarget;
use anyhow::Context as _;
use rayon::prelude::*;
use tracing::{error, info, warn};

/// Whether the `command`/`coverage` phase would run for `lang_test` under `coverage`.
fn resolve_command_list(lang_test: &TestConfig, coverage: bool) -> Option<&StringOrVec> {
    if coverage {
        lang_test.coverage.as_ref().or(lang_test.command.as_ref())
    } else {
        lang_test.command.as_ref()
    }
}

/// Check the precondition gate for the `e2e` phase.
///
/// `precondition` is written for what `command`/`before` need, not for `e2e` -- a block that
/// tests Python via `uv` but lints with `ruff` has no reason for its `e2e` run to require `ruff`.
/// Gating `e2e` on the block's main `precondition` inherits a check that was authored for a
/// different command, which silently skips a suite that could otherwise run.
///
/// `e2e_precondition` scopes the tooling check to what `e2e` itself needs. When it is unset, this
/// deliberately does NOT fall back to `precondition`: falling back would reintroduce the exact bug
/// this field exists to fix. Instead `e2e` runs ungated. The failure mode this trades away is a
/// hard command failure (e.g. "napi: command not found") instead of a graceful skip -- worse
/// diagnostics, but not a false "everything passed" and not a silent no-op either; the missing
/// tool surfaces as a real, reported test failure. Blocks that want a graceful skip declare
/// `e2e_precondition` explicitly; `validate_test_e2e_precondition` requires one of the two
/// precondition fields whenever `e2e` is set, so this ungated path only applies when a
/// `precondition` covers the `command`/`before` phase but was never meant to gate `e2e` too.
fn check_e2e_precondition(lang: Language, lang_test: &TestConfig) -> bool {
    check_precondition(lang, lang_test.e2e_precondition.as_deref())
}

pub fn test(config: &ResolvedCrateConfig, languages: &[Language], e2e: bool, coverage: bool) -> anyhow::Result<()> {
    let pdfium_dir = compute_pdfium_dir();
    let mut env_vars: Vec<(&str, String)> = Vec::new();

    if let Some(lib_dir) = pdfium_dir {
        #[cfg(target_os = "macos")]
        {
            env_vars.push(("DYLD_FALLBACK_LIBRARY_PATH", lib_dir.clone()));
            env_vars.push(("DYLD_LIBRARY_PATH", lib_dir));
        }
        #[cfg(target_os = "linux")]
        {
            env_vars.push(("LD_LIBRARY_PATH", lib_dir));
        }
        #[cfg(target_os = "windows")]
        {
            env_vars.push(("PATH", lib_dir));
        }
    }

    let base_dir = std::env::current_dir()?;

    let langs_to_test: Vec<Language> = languages
        .iter()
        .copied()
        .filter(|lang| {
            let lang_test = config.test_config_for_language(*lang);
            let command_will_run = resolve_command_list(&lang_test, coverage).is_some();
            let e2e_will_run = e2e && lang_test.e2e.is_some();

            let command_gate = command_will_run && check_precondition(*lang, lang_test.precondition.as_deref());
            let e2e_gate = e2e_will_run && check_e2e_precondition(*lang, &lang_test);

            command_gate || e2e_gate
        })
        .collect();
    ensure_requested_suites_will_run(languages, &langs_to_test)?;
    for &lang in &langs_to_test {
        let lang_test = config.test_config_for_language(lang);
        if let Err(e) = run_before(lang, lang_test.before.as_ref()) {
            return Err(e).with_context(|| format!("before hook failed for {lang}"));
        }
    }

    if e2e {
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() || !check_e2e_precondition(lang, &lang_test) {
                continue;
            }
            let Some(backend) = registry::try_get_backend(lang) else {
                continue;
            };
            let Some(bc) = backend.build_config_with_config(config) else {
                continue;
            };
            if bc.post_build.is_empty() {
                continue;
            }
            if let Err(e) = super::run_post_build(lang, &bc, config, &base_dir) {
                warn!("[{lang}] post-build processing failed before e2e tests: {e}");
                return Err(e).with_context(|| format!("post-build failed for {lang}"));
            }
        }

        let host_target = get_host_target().context("failed to detect host target for FFI staging")?;
        for &lang in &langs_to_test {
            let lang_test = config.test_config_for_language(lang);
            if lang_test.e2e.is_none() || !check_e2e_precondition(lang, &lang_test) {
                continue;
            }
            if !matches!(lang, Language::Go | Language::Java | Language::Csharp) {
                continue;
            }
            let workspace_root = std::env::current_dir().ok().and_then(|cwd| {
                let mut current = cwd;
                loop {
                    if current.join("target").exists() && current.join("alef.toml").exists() {
                        return Some(current);
                    }
                    if !current.pop() {
                        return None;
                    }
                }
            });
            if let Some(workspace_root) = workspace_root {
                match ffi_stage::stage_ffi(config, lang, &host_target, &workspace_root) {
                    Ok(dest) => {
                        info!("[{lang}] staged FFI artifacts to {}", dest.display());
                    }
                    Err(e) => {
                        warn!("[{lang}] failed to stage FFI artifacts: {e}");
                    }
                }
                if let Ok(Some(header)) = ffi_stage::stage_header(config, lang, &host_target, &workspace_root) {
                    info!("[{lang}] staged FFI header to {}", header.display());
                }
            }
        }
    }

    let results: Vec<(Language, anyhow::Result<()>)> = langs_to_test
        .par_iter()
        .map(|lang| {
            let label = lang.to_string();
            let lang_test = config.test_config_for_language(*lang);

            let test_cmds = resolve_command_list(&lang_test, coverage);

            if let Some(cmd_list) = test_cmds
                && check_precondition(*lang, lang_test.precondition.as_deref())
            {
                for cmd in cmd_list.commands() {
                    if let Err(e) = run_command_streamed_with_env(cmd, Some(&label), &env_vars) {
                        return (*lang, Err(e));
                    }
                }
            }
            if e2e
                && let Some(e2e_cmd_list) = &lang_test.e2e
                && check_e2e_precondition(*lang, &lang_test)
            {
                for cmd in e2e_cmd_list.commands() {
                    if let Err(e) = run_command_streamed_with_env(cmd, Some(&label), &env_vars) {
                        return (*lang, Err(e));
                    }
                }
            }
            (*lang, Ok(()))
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (lang, result) in results {
        if let Err(e) = result {
            error!("test failed: {lang} — {e}");
            if first_error.is_none() {
                first_error = Some(e);
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

fn ensure_requested_suites_will_run(requested: &[Language], selected: &[Language]) -> anyhow::Result<()> {
    if !requested.is_empty() && selected.is_empty() {
        anyhow::bail!("every requested test suite was skipped by its precondition");
    }
    Ok(())
}

/// Compute the target/release directory path from the workspace root.
///
/// Walks up from the current working directory to find any `target/release/`
/// containing a dynamic library (regardless of which one — pdfium, an FFI crate,
/// etc.). This directory is added to the dynamic library search path so that
/// e2e test runners (dotnet test, JVM, Node, …) can dlopen the workspace's
/// native libraries.
fn compute_pdfium_dir() -> Option<String> {
    use std::env;

    let (lib_prefix, lib_ext): (&str, &str) = if cfg!(target_os = "macos") {
        ("lib", ".dylib")
    } else if cfg!(target_os = "windows") {
        ("", ".dll")
    } else {
        ("lib", ".so")
    };

    let mut current = env::current_dir().ok()?;

    loop {
        let target_release = current.join("target").join("release");
        if target_release.exists()
            && let Ok(entries) = std::fs::read_dir(&target_release)
        {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else { continue };
                if name_str.starts_with(lib_prefix)
                    && name_str.ends_with(lib_ext)
                    && let Some(path_str) = target_release.to_str()
                {
                    info!("Native library directory: {}", path_str);
                    return Some(path_str.to_string());
                }
            }
        }

        if !current.pop() {
            break;
        }
    }

    None
}

/// Build the `rustc --version --verbose` command used by [`get_host_target`], pinned to
/// [`std::env::temp_dir`] rather than left to inherit the ambient process working directory.
///
/// This crate's test binary runs every `#[test]` as a thread in one process, and other tests
/// move the process-wide cwd into a tempdir that is later deleted (see
/// `crate::test_support::CwdGuard`). An unpinned `rustc` spawn that races one of those inherits
/// a deleted directory and fails with "Could not locate working directory" -- a failure with
/// nothing to do with the code under test. `rustc --version --verbose` reads nothing relative to
/// its cwd, so any directory guaranteed to exist for the process lifetime works; the system temp
/// directory decouples this call from process-global state entirely, matching 22baa34ac's fix
/// for the same race in the compile-harness tests. Split out from [`get_host_target`] so the
/// pin can be asserted directly on the built [`std::process::Command`] (see
/// `tests::rustc_version_command_is_pinned_to_a_stable_directory`) instead of by reproducing a
/// deleted ambient cwd against the live process -- that reproduction corrupted
/// `std::env::current_dir()` for every other thread in the shared test binary, not just this
/// one, which is its own instance of the same race class this pin exists to prevent. ~keep
fn rustc_version_command() -> std::process::Command {
    let mut command = std::process::Command::new("rustc");
    command
        .arg("--version")
        .arg("--verbose")
        .current_dir(std::env::temp_dir());
    command
}

/// Get the current host Rust target triple by parsing rustc output.
///
/// Parses `rustc --version --verbose` to extract the `host:` line,
/// returning the target triple (e.g. `aarch64-apple-darwin`).
fn get_host_target() -> anyhow::Result<RustTarget> {
    let output = rustc_version_command()
        .output()
        .context("failed to run rustc --version --verbose")?;

    if !output.status.success() {
        anyhow::bail!("rustc --version --verbose exited with non-zero status");
    }

    let stdout = String::from_utf8(output.stdout).context("rustc output is not valid UTF-8")?;

    for line in stdout.lines() {
        if let Some(triple) = line.strip_prefix("host:") {
            let triple = triple.trim();
            return RustTarget::parse(triple).with_context(|| format!("failed to parse host target triple: {triple}"));
        }
    }

    anyhow::bail!("rustc --version --verbose did not output a 'host:' line")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::core::config::NewAlefConfig;

    /// Guards the fix for the exact race class fixed for the compile-harness tests in
    /// 22baa34ac: an unpinned `rustc` spawn that inherits a deleted ambient cwd fails with
    /// "Could not locate working directory", a failure with nothing to do with the code under
    /// test.
    ///
    /// Asserts the pin directly on the built [`std::process::Command`] rather than by
    /// reproducing a deleted ambient cwd against the live process (entering a tempdir as the
    /// process cwd, deleting it out from under that cwd, then calling `get_host_target`, as an
    /// earlier version of this test did). `std::env::set_current_dir` and
    /// `std::env::current_dir` are process-global, not per-thread: deleting the directory that
    /// backs the live process cwd makes `current_dir()` fail for *every* thread in this shared
    /// test binary for as long as the deletion is in effect, not just the thread running this
    /// test. `crate::test_support::CwdGuard`'s lock only serializes against other lock-holders;
    /// it does not, and structurally cannot, protect the crate's many unguarded
    /// `std::env::current_dir()` readers (for example
    /// `commands::test_apps::start_mock_server`) from observing that corruption. The earlier
    /// version of this test was therefore itself an instance of the very race class it existed
    /// to guard against. Inspecting `Command::get_current_dir()` proves the same property --
    /// that the spawn does not depend on the ambient cwd -- without ever mutating process-global
    /// state. ~keep
    #[test]
    fn rustc_version_command_is_pinned_to_a_stable_directory() {
        let command = rustc_version_command();
        assert_eq!(
            command.get_current_dir(),
            Some(std::env::temp_dir().as_path()),
            "the rustc spawn must be pinned to std::env::temp_dir(), not inherit the ambient cwd"
        );
    }

    /// Build a ResolvedCrateConfig that has `before` and `e2e` wired for `python`
    /// using the given shell commands.  `command` is intentionally absent so the
    /// test exercises the `before` -> `e2e` path in isolation (no unit-test phase).
    #[cfg(unix)]
    fn make_config_with_before_and_e2e(before_cmd: &str, e2e_cmd: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
before = ["{before_cmd}"]
e2e = "{e2e_cmd}"
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// Verify that `before` hooks run before `e2e` commands.
    ///
    /// Before this fix, `before` entries in `[crates.test.<lang>]` were only
    /// documented as running prior to `command`; e2e invocations could start
    /// without the setup steps completing first (e.g. a Kotlin Android binding
    /// can need its FFI library built and symlinked before Gradle loads JNI).
    /// Phase 1 runs `before`
    /// sequentially for every language before Phase 2 executes either
    /// `command` or `e2e`, so both invocation paths receive the same setup.
    #[cfg(unix)]
    #[test]
    fn before_hook_runs_before_e2e_command() {
        let tmp = std::env::temp_dir().join(format!("alef_before_e2e_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create tmp dir");
        let order_file = tmp.join("order.txt");
        std::fs::write(&order_file, "").expect("create order file");

        let path_str = order_file.display().to_string().replace('\'', "'\\''");

        let before_cmd = format!("printf 'A\\n' >> '{path_str}'");
        let e2e_cmd = format!("printf 'B\\n' >> '{path_str}'");

        let config = make_config_with_before_and_e2e(&before_cmd, &e2e_cmd);

        test(&config, &[Language::Python], true, false)
            .expect("test() should succeed when before and e2e commands exit 0");

        let content = std::fs::read_to_string(&order_file).expect("read order file");
        let lines: Vec<&str> = content.lines().collect();

        std::fs::remove_dir_all(&tmp).ok();

        assert_eq!(
            lines,
            vec!["A", "B"],
            "before hook must run before e2e command; got order: {lines:?}"
        );
    }

    /// Build a ResolvedCrateConfig with `[crates.test.python]` set to `extra_toml` plus an `e2e`
    /// command built from `e2e_cmd`. No `command` is set, so the block exercises the `e2e` phase
    /// in isolation -- matching the common consumer shape (`before` + `e2e`, no unit-test phase).
    #[cfg(unix)]
    fn make_config_with_e2e(extra_toml: &str, e2e_cmd: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
{extra_toml}
e2e = "{e2e_cmd}"
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// A shell command that appends `text` to `path`, escaped for single-quoting.
    #[cfg(unix)]
    fn shell_append_cmd(path: &std::path::Path, text: &str) -> String {
        let escaped = path.display().to_string().replace('\'', "'\\''");
        format!("printf '{text}\\n' >> '{escaped}'")
    }

    /// A fresh, empty marker file path scoped by `name` and the test process id, so concurrent
    /// tests in this file never collide.
    #[cfg(unix)]
    fn marker_path(name: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("alef_e2e_precondition_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create tmp dir");
        tmp.join("marker.txt")
    }

    /// `e2e_precondition` present and passing: the e2e command runs.
    #[cfg(unix)]
    #[test]
    fn e2e_precondition_present_and_passing_allows_e2e_to_run() {
        let marker = marker_path("passing");
        let e2e_cmd = shell_append_cmd(&marker, "ran");
        let config = make_config_with_e2e("e2e_precondition = \"true\"", &e2e_cmd);

        test(&config, &[Language::Python], true, false).expect("e2e should run when e2e_precondition passes");

        assert!(marker.exists(), "e2e command should have executed");
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// `e2e_precondition` present and failing: the e2e command is skipped, and since it is the
    /// only requested suite, the exit-1 backstop fires.
    #[cfg(unix)]
    #[test]
    fn e2e_precondition_present_and_failing_skips_e2e_and_trips_backstop() {
        let marker = marker_path("failing");
        let e2e_cmd = shell_append_cmd(&marker, "ran");
        let config = make_config_with_e2e("e2e_precondition = \"false\"", &e2e_cmd);

        let error = test(&config, &[Language::Python], true, false)
            .expect_err("a failing e2e_precondition for the only requested language must trip the backstop");
        assert!(
            error.to_string().contains("every requested test suite was skipped"),
            "got: {error}"
        );
        assert!(!marker.exists(), "e2e command must not run when e2e_precondition fails");
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// `e2e_precondition` absent: the e2e phase runs ungated rather than inheriting a failing main
    /// `precondition` written for a different command. This is the fix for the reported defect --
    /// before it, a block with no `command` (only `before` + `e2e`) had no way to give `e2e` its
    /// own tooling gate, so consumers set `precondition` to whatever `command` needed and `e2e`
    /// inherited it.
    #[cfg(unix)]
    #[test]
    fn absent_e2e_precondition_does_not_inherit_a_failing_main_precondition() {
        let marker = marker_path("absent");
        let e2e_cmd = shell_append_cmd(&marker, "ran");
        let config = make_config_with_e2e("precondition = \"false\"", &e2e_cmd);

        test(&config, &[Language::Python], true, false)
            .expect("e2e must run ungated when e2e_precondition is absent, even if the main precondition fails");

        assert!(
            marker.exists(),
            "e2e command should have executed despite the failing main precondition"
        );
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    /// The main `precondition` still gates the `command` phase exactly as before this change.
    #[cfg(unix)]
    #[test]
    fn main_precondition_still_gates_the_command_phase() {
        let marker = marker_path("command_gate");
        let cmd = shell_append_cmd(&marker, "ran");
        let toml = format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.test.python]
precondition = "false"
command = "{cmd}"
"#
        );
        let cfg: NewAlefConfig = toml::from_str(&toml).unwrap();
        let config = cfg.resolve().unwrap().remove(0);

        let error = test(&config, &[Language::Python], false, false)
            .expect_err("a failing main precondition must still skip the command phase and trip the backstop");
        assert!(
            error.to_string().contains("every requested test suite was skipped"),
            "got: {error}"
        );
        assert!(
            !marker.exists(),
            "command must not run when the main precondition fails"
        );
        std::fs::remove_dir_all(marker.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_all_requested_suites_being_skipped() {
        let error = ensure_requested_suites_will_run(&[Language::Python], &[]).expect_err("zero suites must fail");
        assert!(error.to_string().contains("every requested test suite was skipped"));
    }

    #[test]
    fn permits_an_explicitly_empty_test_selection() {
        ensure_requested_suites_will_run(&[], &[]).expect("no requested suites is valid");
    }
}
