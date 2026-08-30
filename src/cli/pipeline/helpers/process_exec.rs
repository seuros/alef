//! Shared "spawn and stream" execution.
//!
//! Both the shell-command path (`super::run_command_streamed_with_env`) and the argv-only
//! path ([`run_argv_step_streamed`]) fan into [`run_prepared_command`], which owns spawning,
//! output-pumping, and turning a non-zero exit into an error -- neither path re-derives that
//! logic. Split out of `helpers.rs` to keep that file under the repo's 1,000-line cap; this is
//! purely an execution-mechanics extraction; the argv/shell design decisions it implements are
//! documented at each function's call sites. ~keep

use super::pump_lines;
use crate::core::config::output::ArgvStep;
use anyhow::Context as _;
use tracing::info;

/// Run one [`ArgvStep`] of an `ArgvRunConfig` directly via `Command::new(command).args(args)`
/// -- never through a shell.
///
/// This is the argv counterpart to `super::run_command_streamed_with_env`: a generated default
/// (e.g. the Go test-app run, whose module path is a free-form, user-authored config value)
/// builds an `ArgvRunConfig` instead of a shell string precisely so a value like that can
/// never be handed to `sh -c` -- every argument here is one opaque element to the child
/// process, so shell metacharacters inside it are inert. `env_vars` is set via `Command::env`
/// only (no shell text exists here to inline them into).
pub(crate) fn run_argv_step_streamed(
    step: &ArgvStep,
    work_dir: &str,
    env_vars: &[(String, String)],
    label: Option<&str>,
) -> anyhow::Result<()> {
    let description = std::iter::once(step.command.as_str())
        .chain(step.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    info!("Running (argv, cwd={work_dir}): {description}");
    let mut command = std::process::Command::new(&step.command);
    command.args(&step.args).current_dir(work_dir);
    for (key, value) in env_vars {
        command.env(key, value);
    }
    run_prepared_command(command, label, &description)
}

/// Shared tail of every streamed command runner: spawn (or run to completion when `label` is
/// `None`), pump piped output through [`pump_lines`] when labeled, wait, and turn a non-zero
/// exit into an error. `description` is used only for logging/error text, never re-parsed.
pub(super) fn run_prepared_command(
    mut command: std::process::Command,
    label: Option<&str>,
    description: &str,
) -> anyhow::Result<()> {
    let Some(prefix) = label else {
        let status = command
            .status()
            .with_context(|| format!("failed to spawn: {description}"))?;
        if !status.success() {
            anyhow::bail!("Command failed: {description}");
        }
        return Ok(());
    };

    let prefix = format!("[{prefix}] ");
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn: {description}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let p1 = prefix.clone();
    let h_out = stdout.map(|s| std::thread::spawn(move || pump_lines(s, &p1)));
    let p2 = prefix.clone();
    let h_err = stderr.map(|s| std::thread::spawn(move || pump_lines(s, &p2)));

    let status = child
        .wait()
        .with_context(|| format!("failed to wait on: {description}"))?;
    if let Some(h) = h_out {
        let _ = h.join();
    }
    if let Some(h) = h_err {
        let _ = h.join();
    }
    if !status.success() {
        anyhow::bail!("Command failed: {description}");
    }
    Ok(())
}
