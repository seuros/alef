pub mod bash;
pub mod c;
pub mod csharp;
pub mod dart;
pub mod documentation;
pub mod elixir;
pub mod go;
pub mod java;
pub mod json_validator;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod r;
pub mod ruby;
pub mod rust;
pub mod swift;
pub mod toml_validator;
pub mod typescript;
pub mod yaml_validator;
pub mod zig;

use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use std::collections::HashMap;
use std::io::Read;
use std::io::Write;

#[cfg(test)]
pub(crate) fn jvm_toolchain_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub type SnippetValidation = (SnippetStatus, Option<String>);
pub type BatchValidation = Vec<SnippetValidation>;

pub trait SnippetValidator: Send + Sync {
    fn language(&self) -> Language;
    fn is_available(&self) -> bool;
    fn is_available_at(&self, _level: ValidationLevel) -> bool {
        self.is_available()
    }
    /// Validate a snippet at the requested level.
    ///
    /// # Errors
    ///
    /// Returns an error when the validator cannot execute its underlying toolchain.
    fn validate(&self, snippet: &Snippet, level: ValidationLevel, timeout_secs: u64) -> Result<SnippetValidation>;
    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<SnippetValidation> {
        if session.is_some() {
            return Err(crate::snippets::error::Error::Other(format!(
                "{} validator does not support binding-aware sessions",
                self.language()
            )));
        }
        self.validate(snippet, level, timeout_secs)
    }
    fn validate_batch_in_session(
        &self,
        _snippets: &[&Snippet],
        _level: ValidationLevel,
        _timeout_secs: u64,
        _session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        None
    }
    fn max_level(&self) -> ValidationLevel;

    /// The highest level this run's environment can actually reach for `requested`, as opposed
    /// to `max_level`'s fixed per-language ceiling. A validator whose deeper levels depend on a
    /// tool that may not be installed (a real type-checker, for instance) overrides this to
    /// report the level it can genuinely back up right now; the runner treats anything below
    /// `requested` as a real downgrade rather than a capability ceiling, because — unlike
    /// `max_level` — the limit could lift on a different machine. Default: no environmental
    /// limit beyond `max_level`. ~keep
    fn achievable_level(&self, _requested: ValidationLevel) -> ValidationLevel {
        ValidationLevel::Run
    }

    /// Whether the gap `achievable_level` reports for `requested` is structural — no check for
    /// that level is wired up in this validator at all, on any machine — as opposed to
    /// `achievable_level`'s default meaning of a real tool that merely happens to be missing from
    /// this run's environment. The runner exempts a structural gap from `Downgraded` the same way
    /// it exempts `max_level`, because it is unsatisfiable however healthy the environment is; an
    /// environmental gap keeps its `Downgraded` status so a genuinely broken environment is never
    /// silently waved through. Default: environmental, not structural. ~keep
    fn achievable_level_is_structural(&self, _requested: ValidationLevel) -> bool {
        false
    }

    fn is_dependency_error(&self, _error_output: &str) -> bool {
        false
    }

    /// Whether `validate_batch_in_session` is genuinely wired up to run one process for many
    /// snippets, as opposed to the default trait implementation, which always returns `None` and
    /// lets the runner fall back to one process per snippet. The runner uses this to decide
    /// upfront whether a group is a real batch — and should be logged and dispatched as one — or
    /// should skip the batch path entirely and go straight to the per-snippet fallback. Without
    /// this, every language was logged as `Starting batched snippet validation` regardless of
    /// batching support, and a validator that doesn't support it silently fell through to a
    /// codepath that log line never covered, leaving no matching `Finished` event. That is purely
    /// an observability gap in the batch/fallback dispatch path, not a signal about whether the
    /// validator itself ran or hung — a healthy, fully-passing language is just as silent there as
    /// a broken one. ~keep
    /// Whether two snippets of this language may run concurrently within one validation session.
    ///
    /// ~keep Most validators write every file they touch into a `ScratchDir`, which
    /// `tempfile::Builder::tempdir_in` allocates fresh per call, so they are already safe to run
    /// side by side. TypeScript, C# and Java instead write fixed-name files (`snippet.ts` +
    /// `tsconfig.json`, `Program.cs` + `Snippet.csproj`, `<Class>.java` + its `.class` output)
    /// directly into the session's shared fingerprint-keyed workspace, two of them compiling with
    /// `current_dir` set to it -- so concurrent snippets would overwrite each other's sources
    /// mid-compile and silently validate the wrong code. Kotlin keeps its sources in scratch but
    /// truncate-writes a fixed-path Gradle init script into the same shared workspace. Those four
    /// were made shared by `6ee684237`, which introduced the session mutex in the same commit;
    /// the mutex was then applied to every language, serializing validators that never needed it.
    /// A language returning `false` here still gets its own session and caches -- only the
    /// mutual exclusion is dropped.
    fn requires_session_exclusivity(&self) -> bool {
        false
    }

    fn supports_batching(&self) -> bool {
        false
    }
}

pub struct ValidatorRegistry {
    validators: HashMap<Language, Box<dyn SnippetValidator>>,
}

impl ValidatorRegistry {
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            validators: HashMap::new(),
        };

        registry.register(Box::new(rust::RustValidator));
        registry.register(Box::new(python::PythonValidator));
        registry.register(Box::new(typescript::TypeScriptValidator));
        registry.register(Box::new(php::PhpValidator));
        registry.register(Box::new(ruby::RubyValidator));
        registry.register(Box::new(elixir::ElixirValidator));
        registry.register(Box::new(bash::BashValidator));
        registry.register(Box::new(toml_validator::TomlValidator));
        registry.register(Box::new(c::CValidator));
        registry.register(Box::new(csharp::CsharpValidator));
        registry.register(Box::new(dart::DartValidator));
        registry.register(Box::new(go::GoValidator));
        registry.register(Box::new(java::JavaValidator));
        registry.register(Box::new(kotlin::KotlinValidator));
        registry.register(Box::new(swift::SwiftValidator));
        registry.register(Box::new(zig::ZigValidator));
        registry.register(Box::new(json_validator::JsonValidator));
        registry.register(Box::new(yaml_validator::YamlValidator));
        registry.register(Box::new(r::RValidator));
        registry.register(Box::new(documentation::TextValidator));
        registry.register(Box::new(documentation::MermaidValidator));
        registry.register(Box::new(documentation::PowerShellValidator));
        registry.register(Box::new(documentation::XmlValidator));
        registry.register(Box::new(documentation::DockerValidator));

        registry
    }

    pub(crate) fn register(&mut self, validator: Box<dyn SnippetValidator>) {
        self.validators.insert(validator.language(), validator);
    }

    #[must_use]
    pub fn get(&self, language: Language) -> Option<&dyn SnippetValidator> {
        self.validators.get(&language).map(Box::as_ref)
    }

    /// Every language this registry can validate, sorted.
    ///
    /// Exists so contracts that must hold for *all* languages — the scratch destination above all,
    /// since the defect it guards against was runners disagreeing with each other — can be
    /// asserted over the registered set rather than over a hand-written list that silently stops
    /// covering the next language someone registers. ~keep
    #[must_use]
    pub fn languages(&self) -> Vec<Language> {
        let mut languages: Vec<Language> = self.validators.keys().copied().collect();
        languages.sort_unstable();
        languages
    }
}

impl Default for ValidatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_script(
    snippet: &Snippet,
    level: ValidationLevel,
    timeout_secs: u64,
    session: Option<&ValidationSession>,
    suffix: &str,
    program: &str,
    syntax_arguments: &[&str],
) -> Result<(SnippetStatus, Option<String>)> {
    let scratch_dir = session.map(ScratchDir::for_session).transpose()?;
    let mut source = match &scratch_dir {
        Some(dir) => tempfile::Builder::new().suffix(suffix).tempfile_in(dir.path())?,
        None => tempfile::Builder::new().suffix(suffix).tempfile()?,
    };
    source.write_all(snippet.code.as_bytes())?;
    source.flush()?;
    let mut command = std::process::Command::new(program);
    if level == ValidationLevel::Run {
        command.arg(source.path());
    } else {
        command.args(syntax_arguments).arg(source.path());
    }
    if let Some(value) = session {
        value.apply(&mut command);
        command.env("RUBYLIB", &value.working_directory);
        command.env("R_LIBS_USER", &value.working_directory);
    }
    let (success, output) = run_command(&mut command, timeout_secs)?;
    Ok(if success {
        (SnippetStatus::Pass, None)
    } else {
        (SnippetStatus::Fail, Some(output))
    })
}

fn strip_ansi_codes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.next(), Some('[')) {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Run a child process with a timeout and capture combined stdout/stderr.
///
/// # Errors
///
/// Returns an error when the child process cannot be spawned, waited on, or times out.
pub fn run_command(command: &mut std::process::Command, timeout_secs: u64) -> Result<(bool, String)> {
    sanitize_environment(command);
    configure_process_group(command);
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| crate::snippets::error::Error::Other(format!("spawn failed: {err}")))?;
    let stdout = child.stdout.take().map(output_reader);
    let stderr = child.stderr.take().map(output_reader);

    let timeout = std::time::Duration::from_secs(timeout_secs);
    match child.wait_timeout(timeout) {
        Ok(Some(status)) => {
            let output = collect_output(stdout, stderr)?;
            Ok((status.success(), strip_ansi_codes(&output)))
        }
        Ok(None) => {
            kill_process_tree(&mut child);
            let _ = child.wait();
            let _ = collect_output(stdout, stderr);
            Err(crate::snippets::error::Error::Timeout {
                command: format!("{command:?}"),
                timeout_secs,
            })
        }
        Err(err) => {
            kill_process_tree(&mut child);
            let _ = child.wait();
            let _ = collect_output(stdout, stderr);
            Err(crate::snippets::error::Error::Other(format!("wait failed: {err}")))
        }
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut std::process::Command) {}

#[cfg(unix)]
fn kill_process_tree(child: &mut std::process::Child) {
    let process_group = format!("-{}", child.id());
    let killed_group = std::process::Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .status()
        .is_ok_and(|status| status.success());
    if !killed_group {
        let _ = child.kill();
    }
}

#[cfg(not(unix))]
fn kill_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn output_reader(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<std::io::Result<String>> {
    std::thread::spawn(move || {
        let mut output = String::new();
        stream.read_to_string(&mut output)?;
        Ok(output)
    })
}

fn collect_output(
    stdout: Option<std::thread::JoinHandle<std::io::Result<String>>>,
    stderr: Option<std::thread::JoinHandle<std::io::Result<String>>>,
) -> Result<String> {
    let read = |handle: std::thread::JoinHandle<std::io::Result<String>>| {
        handle
            .join()
            .map_err(|_| crate::snippets::error::Error::Other("snippet output reader panicked".into()))?
            .map_err(crate::snippets::error::Error::from)
    };
    let mut output = stdout.map(read).transpose()?.unwrap_or_default();
    output.push_str(&stderr.map(read).transpose()?.unwrap_or_default());
    Ok(output)
}

const SANITIZED_ENVIRONMENT_VARIABLES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "WINDIR",
    "TMP",
    "TEMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "GOMODCACHE",
    "GOPATH",
];

/// The variables that identify the machine itself on Windows, allowed through in addition to
/// [`SANITIZED_ENVIRONMENT_VARIABLES`].
///
/// Sanitisation clears the child environment and re-adds an allowlist. That allowlist was
/// Unix-shaped, and on Windows a toolchain that cannot see these does not degrade -- it fails
/// with an error that names none of them. NuGet resolves its global packages folder through
/// `USERPROFILE`, and without it every `dotnet build` dies in `NuGet.targets` with
/// `Value cannot be null. (Parameter 'path1')`. rustc locates the MSVC linker by running
/// `vswhere.exe` under `ProgramFiles(x86)`, and without it falls back to the first `link.exe`
/// on `PATH` -- which on any box with Git for Windows is GNU coreutils' `link`, producing
/// `link: extra operand` and advice to install the C++ build tools that are already installed.
/// These are the same class of variable as `SYSTEMROOT` and `WINDIR`, which the list above
/// already allows, and they carry no consumer-specific state. ~keep
const WINDOWS_ENVIRONMENT_VARIABLES: &[&str] = &[
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ALLUSERSPROFILE",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "COMSPEC",
    "SystemDrive",
    "PUBLIC",
    "USERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
];

fn sanitize_environment(command: &mut std::process::Command) {
    apply_environment_allowlist(command, cfg!(windows), |key| std::env::var_os(key));
}

/// Replace `command`'s inherited environment with the allowlisted subset `lookup` can resolve,
/// keeping any variable the caller set explicitly.
///
/// `include_windows_variables` and `lookup` are parameters rather than reads of the ambient
/// platform and environment so the allowlist can be asserted on any host: a test that has to
/// mutate the real process environment to check this would be racing every other test in the
/// binary. ~keep
fn apply_environment_allowlist(
    command: &mut std::process::Command,
    include_windows_variables: bool,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) {
    let windows_variables = include_windows_variables
        .then_some(WINDOWS_ENVIRONMENT_VARIABLES)
        .unwrap_or_default();
    let values: Vec<_> = SANITIZED_ENVIRONMENT_VARIABLES
        .iter()
        .chain(windows_variables)
        .filter_map(|key| lookup(key).map(|value| (*key, value)))
        .collect();
    let explicit_values = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key.to_os_string(), value.to_os_string())))
        .collect::<Vec<_>>();
    command.env_clear();
    command.envs(values);
    command.envs(explicit_values);
    command.env("NO_COLOR", "1");
}

trait WaitTimeout {
    fn wait_timeout(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
}

/// The first gap between `try_wait` polls. A fixed 50ms interval charged every subprocess about
/// 25ms of pure sleep on average — invisible for one `cargo check`, tens of seconds across a run
/// with thousands of snippets, because most snippet toolchain invocations finish in single-digit
/// milliseconds. ~keep
const INITIAL_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// The ceiling the backoff grows to, so a genuinely long compile still costs at most one wakeup
/// per 50ms rather than a thousand. ~keep
const MAX_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Doubles a poll interval up to [`MAX_WAIT_POLL_INTERVAL`].
fn next_poll_interval(current: std::time::Duration) -> std::time::Duration {
    current
        .checked_mul(2)
        .unwrap_or(MAX_WAIT_POLL_INTERVAL)
        .min(MAX_WAIT_POLL_INTERVAL)
}

impl WaitTimeout for std::process::Child {
    fn wait_timeout(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        let mut poll_interval = INITIAL_WAIT_POLL_INTERVAL;

        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Ok(None);
            }

            std::thread::sleep(poll_interval.min(timeout - elapsed));
            poll_interval = next_poll_interval(poll_interval);
        }
    }
}

#[cfg(test)]
mod environment_tests {
    use std::collections::HashMap;
    use std::ffi::OsString;

    /// Every allowlisted name mapped to a recognisable value, so a dropped variable shows up as a
    /// missing key rather than as an empty string that could have come from anywhere.
    fn fake_environment() -> HashMap<&'static str, OsString> {
        super::SANITIZED_ENVIRONMENT_VARIABLES
            .iter()
            .chain(super::WINDOWS_ENVIRONMENT_VARIABLES)
            .map(|key| (*key, OsString::from(format!("value-of-{key}"))))
            .collect()
    }

    fn sanitized(include_windows_variables: bool) -> HashMap<String, String> {
        let environment = fake_environment();
        let mut command = std::process::Command::new("does-not-run");
        command.env("EXPLICIT", "kept");
        super::apply_environment_allowlist(&mut command, include_windows_variables, |key| {
            environment.get(key).cloned()
        });
        command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()))
            })
            .collect()
    }

    #[test]
    fn go_dependency_cache_paths_survive_sanitisation() {
        let passed = sanitized(false);

        assert_eq!(
            passed.get("GOMODCACHE").map(String::as_str),
            Some("value-of-GOMODCACHE")
        );
        assert_eq!(passed.get("GOPATH").map(String::as_str), Some("value-of-GOPATH"));
    }

    /// The two variables named here are the ones the Windows CI failures traced back to, and they
    /// are asserted individually rather than as "the list is non-empty" because dropping either
    /// one on its own is a whole language going dark: `USERPROFILE` for `dotnet`, and
    /// `ProgramFiles(x86)` for rustc's MSVC linker discovery. ~keep
    #[test]
    fn windows_toolchain_variables_survive_sanitisation_on_windows_hosts() {
        let passed = sanitized(true);

        assert_eq!(
            passed.get("USERPROFILE").map(String::as_str),
            Some("value-of-USERPROFILE"),
            "dotnet restore resolves its global packages folder through USERPROFILE"
        );
        assert_eq!(
            passed.get("ProgramFiles(x86)").map(String::as_str),
            Some("value-of-ProgramFiles(x86)"),
            "rustc finds vswhere.exe, and so link.exe, under ProgramFiles(x86)"
        );
        for key in super::WINDOWS_ENVIRONMENT_VARIABLES {
            assert!(passed.contains_key(*key), "{key} must survive sanitisation");
        }
    }

    /// The Windows names must not widen what a Unix child inherits: `USERNAME` and `PUBLIC` do
    /// exist on some Unix hosts, and sanitisation is an isolation boundary, not a convenience. ~keep
    #[test]
    fn windows_variables_are_withheld_from_non_windows_hosts() {
        let passed = sanitized(false);

        for key in super::WINDOWS_ENVIRONMENT_VARIABLES {
            assert!(
                !passed.contains_key(*key),
                "{key} must not leak into a non-Windows child"
            );
        }
    }

    #[test]
    fn explicitly_set_variables_outlive_the_environment_clear() {
        let passed = sanitized(true);

        assert_eq!(passed.get("EXPLICIT").map(String::as_str), Some("kept"));
        assert_eq!(passed.get("NO_COLOR").map(String::as_str), Some("1"));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::{Duration, Instant};

    /// The backoff has to start far below the old fixed 50ms floor and still stop growing, so a
    /// short command returns almost immediately while a long compile is not polled a thousand
    /// times a second. ~keep
    ///
    /// ~keep This is the whole coverage for that property, deliberately. A companion test used to
    /// time 20 trivial `sh -c 'exit 0'` runs and assert the amortised cost stayed under the old
    /// fixed interval, but bare process-spawn overhead on a loaded machine reaches 60ms/command --
    /// more than the 50ms bound it was trying to prove we no longer pay -- so it failed on load
    /// rather than on regression, at two successive thresholds. Asserting the schedule directly
    /// proves the same thing and cannot be perturbed by what else the machine is doing. Do not
    /// re-add a wall-clock version.
    #[test]
    fn the_wait_backoff_starts_at_one_millisecond_and_caps_at_fifty() {
        assert_eq!(super::INITIAL_WAIT_POLL_INTERVAL, Duration::from_millis(1));

        let intervals = std::iter::successors(Some(super::INITIAL_WAIT_POLL_INTERVAL), |current| {
            Some(super::next_poll_interval(*current))
        })
        .take(8)
        .collect::<Vec<_>>();

        assert_eq!(
            intervals,
            vec![
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(4),
                Duration::from_millis(8),
                Duration::from_millis(16),
                Duration::from_millis(32),
                Duration::from_millis(50),
                Duration::from_millis(50),
            ]
        );
    }

    #[test]
    fn drains_output_larger_than_an_os_pipe_buffer() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "dd if=/dev/zero bs=131072 count=1 2>/dev/null"]);

        let (success, output) = super::run_command(&mut command, 5).expect("large-output command");

        assert!(success);
        assert_eq!(output.len(), 131_072);
    }

    #[test]
    fn timeout_kills_descendants_holding_output_pipes() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        let started = Instant::now();

        let error = super::run_command(&mut command, 1).expect_err("command must time out");

        assert!(matches!(error, crate::snippets::error::Error::Timeout { .. }));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    fn script_session(working_directory: std::path::PathBuf) -> crate::snippets::session::ValidationSession {
        crate::snippets::session::ValidationSession {
            language: crate::snippets::types::Language::Bash,
            working_directory,
            manifest: None,
            fingerprint: "run-script-scratch-fixture".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        }
    }

    fn script_snippet(code: &str) -> crate::snippets::types::Snippet {
        crate::snippets::types::Snippet {
            id: None,
            path: "example.md".into(),
            language: crate::snippets::types::Language::Bash,
            title: None,
            code: code.to_string(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: crate::snippets::types::SnippetMetadata::default(),
            source_origin: crate::snippets::types::SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    /// Regression: `run_script` (shared by bash/php/r/ruby) used to write its scratch file
    /// directly into `session.working_directory` via a bare `tempfile_in`, producing an
    /// untracked `.tmp<random><suffix>` file with no `.gitignore` coverage — the exact shape of
    /// the git-visible litter this fix closes. It must resolve under the session's own cache
    /// tree instead, leaving nothing behind at the top level of `working_directory` at all. ~keep
    #[test]
    fn run_script_resolves_scratch_under_the_cache_root_not_directly_in_working_directory() {
        let working = tempfile::tempdir().expect("working directory");
        let session = script_session(working.path().to_path_buf());
        let snippet = script_snippet("true\n");

        let (status, _) = super::run_script(
            &snippet,
            super::ValidationLevel::Syntax,
            5,
            Some(&session),
            ".sh",
            "true",
            &[],
        )
        .expect("run_script runs");

        assert_eq!(status, super::SnippetStatus::Pass);
        let top_level_entries: Vec<_> = std::fs::read_dir(working.path())
            .expect("read working directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect();
        assert!(
            top_level_entries.is_empty(),
            "run_script must not leave any scratch entry directly in working_directory: {top_level_entries:?}"
        );
    }

    /// Pins cleanup on the failure path specifically: a snippet that fails validation must not
    /// leave its scratch directory behind any more than a passing one does.
    #[test]
    fn run_script_removes_scratch_after_a_run_that_fails() {
        let working = tempfile::tempdir().expect("working directory");
        let session = script_session(working.path().to_path_buf());
        let snippet = script_snippet("false\n");

        let (status, _) = super::run_script(
            &snippet,
            super::ValidationLevel::Syntax,
            5,
            Some(&session),
            ".sh",
            "false",
            &[],
        )
        .expect("run_script runs");

        assert_eq!(status, super::SnippetStatus::Fail);
        let scratch_root = working.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a failing snippet validation"
        );
    }

    /// The load-bearing exit path, and the one explicit cleanup calls always missed: `run_script`
    /// returns `Err` from `?` — here because the toolchain is not installed at all, elsewhere
    /// because the child timed out — long before any cleanup statement at the bottom of the
    /// function could run. Only a `Drop` guard covers this, so if scratch ever survives here the
    /// mechanism has silently regressed to explicit cleanup. ~keep
    #[test]
    fn run_script_removes_scratch_when_the_run_returns_an_error() {
        let working = tempfile::tempdir().expect("working directory");
        let session = script_session(working.path().to_path_buf());
        let snippet = script_snippet("true\n");

        let error = super::run_script(
            &snippet,
            super::ValidationLevel::Syntax,
            5,
            Some(&session),
            ".sh",
            "alef-nonexistent-toolchain-for-scratch-test",
            &[],
        )
        .expect_err("a missing toolchain must surface as an error, not a status");

        assert!(matches!(error, crate::snippets::error::Error::Other(_)));
        let scratch_root = session.scratch_root();
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        assert_eq!(
            remaining,
            0,
            "scratch left behind under {} after run_script returned an error",
            scratch_root.display()
        );
    }
}
