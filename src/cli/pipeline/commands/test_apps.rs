use crate::cli::cache;
use crate::cli::pipeline::helpers::{check_precondition_named, run_command_streamed};
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash;
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{error, info, warn};

/// Names in `names` whose `<registry output>/<name>/` directory exists, when `config`'s
/// crate-wide generation-inputs fingerprint no longer matches what
/// `cache::generation_record` recorded for it at its last successful `alef generate`/`alef
/// all` run.
///
/// Reuses the exact central record `alef verify` compares against
/// (`cache::generation_record::recorded_inputs_hash`) instead of deriving a second
/// staleness signal, so this can never disagree with what `alef verify` would find for the
/// crate. A registry-mode test app that fails while running against stale generated
/// sources reads exactly like a real regression; this exists so `test_apps_run` can name
/// the actual cause up front instead of leaving an operator to chase a wrong-cause failure
/// through the harness.
///
/// Deliberately crate-scoped, not per-file: `core::hash::compute_file_hash` no longer takes
/// a generation-inputs argument (see its doc), so there is no per-file staleness signal left
/// to walk test-app directories for. A crate with no recorded baseline yet (every consumer,
/// immediately after upgrading to this version) is not staleness — this only reports a crate
/// it could actually compare against a real baseline, matching `verify`'s own "examined
/// nothing" caution. ~keep
fn stale_test_app_names(
    config: &ResolvedCrateConfig,
    config_path: &Path,
    base_dir: &Path,
    names: &[String],
) -> Vec<String> {
    let Some(e2e) = config.e2e.as_ref() else {
        return Vec::new();
    };
    let Ok(sources_hash) = cache::sources_hash(&config.sources) else {
        return Vec::new();
    };
    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);
    let inputs_hash = hash::compute_inputs_hash(&sources_hash, &alef_toml_bytes);
    let Some(recorded) = cache::recorded_inputs_hash(base_dir, &config.name) else {
        return Vec::new();
    };
    if recorded == inputs_hash {
        return Vec::new();
    }

    let output_root = base_dir.join(&e2e.registry.output);
    names
        .iter()
        .filter(|name| output_root.join(name).is_dir())
        .cloned()
        .collect()
}

/// Log one `tracing::warn!` per stale target named by [`stale_test_app_names`], naming the
/// exact remedy command so a subsequent test-app failure is diagnosable at the top of the
/// run instead of inferred from a confusing failure deep in the harness. Never blocks the
/// run: a registry-mode test app pinned to an intentionally older tag is a legitimate use
/// that must not be forced to regenerate first. ~keep
fn warn_if_test_apps_stale(config: &ResolvedCrateConfig, config_path: &Path, names: &[String]) {
    let Ok(base_dir) = std::env::current_dir() else {
        return;
    };
    for name in stale_test_app_names(config, config_path, &base_dir, names) {
        warn!(
            "test-app '{name}' output looks stale: sources or alef.toml changed since these files \
             were generated. A failure below may be this, not a real regression — run `alef \
             test-apps generate` (or `alef all`) first, then re-run `alef test-apps run`."
        );
    }
}

/// Outcome of running a single language's registry-mode test app.
///
/// Distinguishes a precondition skip from a genuine pass so the summary can
/// report them separately (a skip is not a failure, but it is also not a pass).
#[derive(Debug)]
enum TestAppOutcome {
    /// The run commands executed successfully.
    Passed,
    /// The precondition command failed, so the run was skipped.
    Skipped,
    /// A before hook or a run command failed.
    Failed(anyhow::Error),
}

/// A running e2e mock-server process plus the env vars its startup line exported.
///
/// The child is kept alive for the lifetime of this guard; dropping it closes the
/// child's stdin (the mock-server blocks reading stdin and exits on EOF) and then
/// kills + reaps the process so no orphan listener survives the run.
struct MockServerHandle {
    child: std::process::Child,
    /// Env vars to inject into every test-app `run` command:
    /// - `MOCK_SERVER_URL` (always)
    /// - `MOCK_SERVERS` JSON map (when the server printed it)
    /// - `MOCK_SERVER_<FIXTURE_ID_UPPER>` per host-root fixture (derived from
    ///   the `MOCK_SERVERS` JSON), so generated shell-based test scripts can
    ///   reference per-fixture URLs without parsing JSON themselves.
    env_vars: Vec<(String, String)>,
}

impl Drop for MockServerHandle {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Check if the given Cargo.toml defines a [[bin]] target named "mock-server".
fn has_mock_server_bin(manifest_path: &std::path::Path) -> anyhow::Result<bool> {
    let content = std::fs::read_to_string(manifest_path)
        .context("failed to read Cargo.toml to check for mock-server bin target")?;
    Ok(content.contains("[[bin]]") && content.contains("name = \"mock-server\""))
}

/// Build and start the shared e2e mock-server, returning a handle whose env vars
/// (`MOCK_SERVER_URL`, optional `MOCK_SERVERS`) must be injected into every
/// test-app `run` command.
///
/// The mock-server crate is the alef-generated `<e2e.output>/rust` project, built
/// in release (mirroring sample_project's Taskfile `e2e:build`), producing the
/// `mock-server` binary at `<e2e.output>/rust/target/release/mock-server`. On
/// startup the binary prints `MOCK_SERVER_URL=http://127.0.0.1:<port>` (and, when
/// host-root fixtures exist, `MOCK_SERVERS={...}`) to stdout, then blocks reading
/// stdin until the parent closes the pipe.
///
/// Returns `Ok(None)` when the e2e config has no fixtures directory / rust crate
/// to build (no HTTP fixtures → no mock-server needed); the test apps then run
/// without the env vars exactly as before. Any build/spawn/parse failure is a hard
/// error so a missing server never silently degrades to "connection refused".
fn start_mock_server(config: &ResolvedCrateConfig) -> anyhow::Result<Option<MockServerHandle>> {
    let Some(e2e) = config.e2e.as_ref() else {
        return Ok(None);
    };
    let base_dir = std::env::current_dir().context("failed to resolve current directory")?;
    let rust_crate_dir = base_dir.join(&e2e.output).join("rust");
    let manifest_path = rust_crate_dir.join("Cargo.toml");
    if !manifest_path.exists() {
        info!(
            "No e2e mock-server crate at {} — running test apps without MOCK_SERVER_URL",
            manifest_path.display()
        );
        return Ok(None);
    }

    if !has_mock_server_bin(&manifest_path)? {
        info!(
            "No [[bin]] mock-server target in {} — running test apps without MOCK_SERVER_URL",
            manifest_path.display()
        );
        return Ok(None);
    }

    info!("Building e2e mock-server: {}", manifest_path.display());
    run_command_streamed(
        &format!(
            "cargo build --release --manifest-path {} --bin mock-server",
            manifest_path.display()
        ),
        Some("mock-server"),
    )
    .context("failed to build the e2e mock-server")?;

    let bin_path = rust_crate_dir.join("target").join("release").join("mock-server");
    if !bin_path.exists() {
        anyhow::bail!("e2e mock-server binary not found after build: {}", bin_path.display());
    }

    let fixtures_dir = base_dir.join(&e2e.fixtures);

    info!(
        "Starting e2e mock-server ({}) with fixtures {}",
        bin_path.display(),
        fixtures_dir.display()
    );
    let mut child = std::process::Command::new(&bin_path)
        .arg(&fixtures_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to spawn e2e mock-server: {}", bin_path.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("e2e mock-server stdout pipe was not captured")?;

    let mut reader = std::io::BufReader::new(stdout);
    let mut url: Option<String> = None;
    let mut servers: Option<String> = None;
    {
        use std::io::BufRead as _;
        let mut line = String::new();
        for _ in 0..8 {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("MOCK_SERVER_URL=") {
                        url = Some(rest.to_string());
                    } else if let Some(rest) = trimmed.strip_prefix("MOCK_SERVERS=") {
                        servers = Some(rest.to_string());
                        break;
                    } else if url.is_some() {
                        break;
                    }
                }
            }
        }
    }

    let url = url
        .context("e2e mock-server did not print a MOCK_SERVER_URL= line on startup; cannot run test apps without it")?;
    info!("e2e mock-server ready at {url}");

    std::thread::spawn(move || {
        use std::io::BufRead as _;
        let mut sink = String::new();
        while reader.read_line(&mut sink).map(|n| n > 0).unwrap_or(false) {
            sink.clear();
        }
    });

    let mut env_vars: Vec<(String, String)> = vec![("MOCK_SERVER_URL".to_string(), url)];
    if let Some(servers) = servers {
        match serde_json::from_str::<std::collections::HashMap<String, String>>(&servers) {
            Ok(map) => {
                for (fixture_id, server_url) in &map {
                    env_vars.push((
                        format!("MOCK_SERVER_{}", fixture_id.to_ascii_uppercase()),
                        server_url.clone(),
                    ));
                }
            }
            Err(e) => {
                warn!(
                    "Failed to parse MOCK_SERVERS JSON for per-fixture env-var derivation: {e}. \
                     Shell-based test apps that expect MOCK_SERVER_<FIXTURE_ID> will fall back to \
                     MOCK_SERVER_URL."
                );
            }
        }
        env_vars.push(("MOCK_SERVERS".to_string(), servers));
    }

    Ok(Some(MockServerHandle { child, env_vars }))
}

/// Run the registry-mode test app for each language.
///
/// Each test app exercises the *published* package against the same fixtures the
/// e2e suite uses. `names` are `[e2e].languages` entries — usually language slugs,
/// but also string-only registry targets like `brew`. For each: check the
/// precondition (skip with a warning on failure), run the `before` hook (abort
/// that target on failure), then execute each configured `run` command with live
/// output. The `run` commands `cd` into their own `test_apps/<name>/` directory,
/// so no cwd is supplied here.
///
/// Before running any apps, a single shared e2e mock-server is built and started;
/// its `MOCK_SERVER_URL` (and `MOCK_SERVERS` when present) is injected into every
/// `run`/`before` command's environment. The harnesses use this value instead of
/// spawning their own server (the local `e2e/<lang>/` binary path does not exist
/// under `test_apps/<lang>/`). The server is stopped when this function returns,
/// on success or failure, via the `MockServerHandle` guard.
///
/// Targets run in parallel (mirroring `setup`/`clean`). A precondition skip — and
/// a target with no `run` command (e.g. `ffi`) — is reported distinctly from a
/// pass; the first failing target's error is returned so the process exits non-zero.
///
/// Before anything runs, each target's generated output is checked against the crate's
/// current sources + `alef.toml` (see [`warn_if_test_apps_stale`]) and a stale target
/// gets a loud warning naming the fix — this never blocks the run, only diagnoses it.
pub fn test_apps_run(config: &ResolvedCrateConfig, config_path: &Path, names: &[String]) -> anyhow::Result<()> {
    warn_if_test_apps_stale(config, config_path, names);
    let server = start_mock_server(config).context("failed to start e2e mock-server for test apps")?;
    let server_env: Vec<(String, String)> = server.as_ref().map(|h| h.env_vars.clone()).unwrap_or_default();
    let e2e_env: Vec<(String, String)> = config
        .e2e
        .as_ref()
        .map(|e2e| {
            let mut vars: Vec<(String, String)> = e2e.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            vars.sort();
            vars
        })
        .unwrap_or_default();
    let env_prefix: String = e2e_env
        .iter()
        .chain(server_env.iter())
        .map(|(k, v)| format!("export {k}='{v}'; "))
        .collect();

    let results: Vec<(String, TestAppOutcome)> = names
        .par_iter()
        .map(|name| {
            let cfg = config.test_apps_run_config_for_name(name);
            if !check_precondition_named(name, cfg.precondition.as_deref()) {
                return (name.clone(), TestAppOutcome::Skipped);
            }
            if let Some(before) = &cfg.before {
                for cmd in before.commands() {
                    if let Err(e) = run_command_streamed(&format!("{env_prefix}{cmd}"), Some(name)) {
                        return (name.clone(), TestAppOutcome::Failed(e));
                    }
                }
            }
            match &cfg.run {
                Some(cmd_list) => {
                    for cmd in cmd_list.commands() {
                        if let Err(e) = run_command_streamed(&format!("{env_prefix}{cmd}"), Some(name)) {
                            return (name.clone(), TestAppOutcome::Failed(e));
                        }
                    }
                    (name.clone(), TestAppOutcome::Passed)
                }
                None => (name.clone(), TestAppOutcome::Skipped),
            }
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (name, outcome) in results {
        match outcome {
            TestAppOutcome::Passed => info!("test-app passed: {name}"),
            TestAppOutcome::Skipped => warn!("test-app skipped: {name}"),
            TestAppOutcome::Failed(e) => {
                error!("test-app failed: {name} — {e}");
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

#[cfg(all(test, unix))]
mod test_apps_run_tests {
    use super::*;

    /// A path guaranteed not to resolve to a real `alef.toml`, for tests that exercise
    /// `test_apps_run` behavior unrelated to the staleness check (`read_alef_toml_bytes`
    /// treats a missing file as empty bytes, which is a stable, harmless input here).
    fn no_config_path() -> &'static Path {
        Path::new("test_apps_run_tests_nonexistent_alef.toml")
    }

    fn resolved_config() -> ResolvedCrateConfig {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "false"
run = "false"
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn failing_precondition_is_skipped_not_failed() {
        let config = resolved_config();
        let result = test_apps_run(&config, no_config_path(), &["python".to_string()]);
        assert!(
            result.is_ok(),
            "a precondition skip must be reported as skipped, not failed: {result:?}"
        );
    }

    #[test]
    fn failing_run_command_propagates_error() {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "true"
run = "false"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let result = test_apps_run(&config, no_config_path(), &["python".to_string()]);
        assert!(result.is_err(), "a failing run command must propagate as an error");
    }

    #[test]
    fn passing_run_command_succeeds() {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "true"
run = "true"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let result = test_apps_run(&config, no_config_path(), &["python".to_string()]);
        assert!(result.is_ok(), "a passing run command must succeed: {result:?}");
    }

    #[test]
    fn e2e_env_vars_are_exported_to_run_command() {
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.env]
ALLOW_PRIVATE_NETWORK = "true"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"

[crates.e2e.registry.run.python]
precondition = "true"
run = "test \"$ALLOW_PRIVATE_NETWORK\" = true"
"#,
        )
        .unwrap();
        let config = cfg.resolve().unwrap().remove(0);
        let result = test_apps_run(&config, no_config_path(), &["python".to_string()]);
        assert!(
            result.is_ok(),
            "a declared [crates.e2e.env] var must reach the run command: {result:?}"
        );
    }

    /// Builds file content carrying a real alef header, marker, and an `alef:hash:` line --
    /// mirroring exactly what `finalize_hashes` writes, so fixtures below look like realistic
    /// test-app output. `stale_test_app_names` no longer reads this file's own stamp (see its
    /// doc: `core::hash::compute_file_hash` takes no generation-inputs argument any more), only
    /// whether the directory containing it exists -- the staleness signal itself comes from
    /// `cache::record_inputs_hash`/`cache::recorded_inputs_hash`.
    fn marked_file(body: &str) -> String {
        let with_header = format!("{}{body}", hash::header(hash::CommentStyle::DoubleSlash));
        let file_hash = hash::compute_file_hash(&with_header);
        hash::inject_hash_line(&with_header, &file_hash)
    }

    /// Regression for #134: `test_apps_run` must be able to tell an operator that a target's
    /// generated output predates the crate's current sources/config, rather than staying
    /// silent and letting a stale-source failure masquerade as a real regression.
    #[test]
    fn stale_test_app_names_flags_outdated_target_and_clears_after_regeneration() {
        // `sources_hash` and `read_alef_toml_bytes` resolve against the process cwd, so a
        // concurrently-running cwd-mutating test changes what this computes between the stale
        // and fresh halves and the fresh stamp stops matching. Passes alone, fails in the full
        // suite. ~keep
        let _guard = crate::test_support::CWD_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let config = resolved_config();
        let registry_output = &config.e2e.as_ref().expect("e2e config").registry.output;
        let target_dir = base.join(registry_output).join("python");
        std::fs::create_dir_all(&target_dir).expect("mkdir");
        let target_file = target_dir.join("app.py");
        std::fs::write(&target_file, marked_file("print('old')\n")).expect("write test-app output");

        // A recorded baseline that cannot match today's sources + alef.toml.
        cache::record_inputs_hash(base, &config.name, &"0".repeat(64)).expect("seed stale baseline");
        let stale = stale_test_app_names(&config, no_config_path(), base, &["python".to_string()]);
        assert_eq!(
            stale,
            vec!["python".to_string()],
            "an outdated recorded baseline must flag its target as stale"
        );

        // Regenerate: record exactly what today's sources_hash + alef.toml bytes produce.
        let sources_hash = cache::sources_hash(&config.sources).expect("sources hash");
        let inputs_hash = hash::compute_inputs_hash(&sources_hash, &cache::read_alef_toml_bytes(no_config_path()));
        cache::record_inputs_hash(base, &config.name, &inputs_hash).expect("record fresh baseline");
        let fresh = stale_test_app_names(&config, no_config_path(), base, &["python".to_string()]);
        assert!(
            fresh.is_empty(),
            "a baseline matching current inputs must not be reported stale: {fresh:?}"
        );
    }

    /// The migration-graceful half: a crate with no recorded baseline yet -- every crate in
    /// every consumer repo immediately after upgrading to this version, before the first `alef
    /// generate`/`alef all` run -- must not be reported stale. Getting this wrong would turn a
    /// silent, advisory pre-flight check into a spurious warning on every upgrade. ~keep
    #[test]
    fn stale_test_app_names_is_silent_with_no_recorded_baseline_yet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let config = resolved_config();
        let registry_output = &config.e2e.as_ref().expect("e2e config").registry.output;
        let target_dir = base.join(registry_output).join("python");
        std::fs::create_dir_all(&target_dir).expect("mkdir");
        std::fs::write(target_dir.join("app.py"), marked_file("print('ok')\n")).expect("write test-app output");

        let stale = stale_test_app_names(&config, no_config_path(), base, &["python".to_string()]);
        assert!(
            stale.is_empty(),
            "a crate with no recorded generation baseline must not be reported stale: {stale:?}"
        );
    }
}
