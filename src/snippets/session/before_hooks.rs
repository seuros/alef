//! Running a session's `before` hooks, and remembering what each one produced.
//!
//! Split out of `session` under the repo's file-size cap.

use super::SessionSpec;
use crate::snippets::error::{Error, Result};
use crate::snippets::validators::run_command;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// A `before` hook invocation, identified by everything the shell it runs in can observe.
///
/// Two sessions in one activation group already share a `working_directory` by construction, so a
/// matching command and environment make the second invocation bit-for-bit the same shell command
/// as the first. ~keep
pub(super) type HookInvocation = (String, BTreeMap<String, String>);

/// What every `before` hook already attempted in this activation group produced, so an invocation
/// indistinguishable from one already made is answered rather than repeated.
///
/// Several configured targets legitimately describe one physical package -- `kotlin` and
/// `kotlin_android` over a single Gradle project, `typescript`/`node`/`wasm` over a single npm
/// package -- and each carries its own copy of that package's `before` hook. Running them all
/// meant one `./gradlew assembleDebug` per target, strictly sequentially, before a single snippet
/// could be validated; when the hook then exceeded `timeout_secs`, the run paid that timeout once
/// per target. Failures are replayed rather than dropped: a hook that failed the first time fails
/// the same way for every session that asked for it, and each of those sessions must still be
/// reported as unprepared. ~keep
pub(super) type HookOutcomes = HashMap<HookInvocation, HookOutcome>;

/// The replayable half of a `before` hook's result. `Error` is not `Clone`, and the distinction
/// between a timeout and any other failure is load-bearing downstream -- see
/// [`SessionPreparationError::ordering`] -- so the variant is preserved rather than the message
/// alone. ~keep
#[derive(Clone)]
pub(super) enum HookOutcome {
    Succeeded,
    TimedOut { command: String, timeout_secs: u64 },
    Failed(String),
}

impl HookOutcome {
    fn record(outcome: &Result<()>) -> Self {
        match outcome {
            Ok(()) => Self::Succeeded,
            Err(Error::Timeout { command, timeout_secs }) => Self::TimedOut {
                command: command.clone(),
                timeout_secs: *timeout_secs,
            },
            Err(other) => Self::Failed(other.to_string()),
        }
    }

    fn replay(&self) -> Result<()> {
        match self {
            Self::Succeeded => Ok(()),
            Self::TimedOut { command, timeout_secs } => Err(Error::Timeout {
                command: command.clone(),
                timeout_secs: *timeout_secs,
            }),
            Self::Failed(message) => Err(Error::Other(message.clone())),
        }
    }
}

/// Runs `source` unless an indistinguishable invocation was already attempted in this activation
/// group, in which case that attempt's outcome is replayed.
pub(super) fn run_before_once(
    source: &str,
    spec: &SessionSpec,
    timeout_secs: u64,
    ran: &mut HookOutcomes,
) -> Result<()> {
    let invocation = (source.to_owned(), spec.env.clone());
    if let Some(previous) = ran.get(&invocation) {
        tracing::debug!(
            command = %source,
            working_directory = %spec.working_directory.display(),
            language = %spec.language,
            "reusing a `before` hook already run for this working directory in this run"
        );
        return previous.replay();
    }
    let outcome = run_before(source, &spec.working_directory, &spec.env, timeout_secs);
    ran.insert(invocation, HookOutcome::record(&outcome));
    outcome
}

fn run_before(source: &str, working_directory: &Path, env: &BTreeMap<String, String>, timeout_secs: u64) -> Result<()> {
    let mut command = shell_command(source);
    command.current_dir(working_directory);
    command.envs(env);
    let (success, output) = run_command(&mut command, timeout_secs)?;
    if success {
        Ok(())
    } else {
        // A failing build hook is the largest diagnostic alef ever reports: it carries a whole
        // build's output, and a hook in a pathological state repeats one warning for every file it
        // wrongly crawled. The bound is applied here rather than in `run_command` because callers
        // that parse a command's output -- Gradle classpath entries, dependency-error matching --
        // must still see all of it. ~keep
        Err(Error::Other(format!(
            "before command failed: {}",
            crate::snippets::diagnostics::bounded_text(&output)
        )))
    }
}

#[cfg(unix)]
fn shell_command(source: &str) -> std::process::Command {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", source]);
    command
}

#[cfg(windows)]
fn shell_command(source: &str) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    command.args(["/C", source]);
    command
}
