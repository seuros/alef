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

    fn is_dependency_error(&self, _error_output: &str) -> bool {
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
    let mut source = match session {
        Some(value) => tempfile::Builder::new()
            .suffix(suffix)
            .tempfile_in(&value.working_directory)?,
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
            let _ = child.kill();
            let _ = child.wait();
            let _ = collect_output(stdout, stderr);
            Err(crate::snippets::error::Error::Timeout {
                command: format!("{command:?}"),
                timeout_secs,
            })
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = collect_output(stdout, stderr);
            Err(crate::snippets::error::Error::Other(format!("wait failed: {err}")))
        }
    }
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

fn sanitize_environment(command: &mut std::process::Command) {
    const ALLOWED_VARIABLES: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "TMP",
        "TEMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
    ];
    let values: Vec<_> = ALLOWED_VARIABLES
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (*key, value)))
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

impl WaitTimeout for std::process::Child {
    fn wait_timeout(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(50);

        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }

            if start.elapsed() >= timeout {
                return Ok(None);
            }

            std::thread::sleep(poll_interval);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn drains_output_larger_than_an_os_pipe_buffer() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "dd if=/dev/zero bs=131072 count=1 2>/dev/null"]);

        let (success, output) = super::run_command(&mut command, 5).expect("large-output command");

        assert!(success);
        assert_eq!(output.len(), 131_072);
    }
}
