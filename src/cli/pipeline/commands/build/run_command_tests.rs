use super::*;
use crate::test_support::SkipCommandsGuard;

#[test]
fn run_run_command_succeeds_for_echo() {
    let _guard = SkipCommandsGuard::set("");
    let dir = std::env::temp_dir();
    let result = run_run_command(
        "echo",
        &["alef-runcommand-ok"],
        &dir,
        "sample",
        super::RUN_COMMAND_TIMEOUT,
    );
    assert!(
        matches!(result, Ok(true)),
        "echo should run and report Ok(true): {result:?}"
    );
}

#[test]
fn run_run_command_fails_for_false() {
    let _guard = SkipCommandsGuard::set("");
    let dir = std::env::temp_dir();
    let result = run_run_command("false", &[], &dir, "sample", super::RUN_COMMAND_TIMEOUT);
    assert!(result.is_err(), "false should return Err");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("exited with status"),
        "error should mention exit status: {msg}"
    );
}

#[test]
fn run_run_command_honors_skip_env_var() {
    let dir = std::env::temp_dir();
    {
        let _guard = SkipCommandsGuard::set("noop,false , another");
        let skipped = run_run_command("false", &[], &dir, "sample", super::RUN_COMMAND_TIMEOUT);
        assert!(
            matches!(skipped, Ok(false)),
            "listed command must return Ok(false) without spawning: {skipped:?}"
        );
    }

    let _guard = SkipCommandsGuard::set("something-else");
    let honored = run_run_command("false", &[], &dir, "sample", super::RUN_COMMAND_TIMEOUT);
    assert!(
        honored.is_err(),
        "unlisted command must still spawn and surface failure"
    );
}

/// A tool missing from `PATH` used to be indistinguishable from a tool that ran and
/// produced current output -- both returned `Ok(())`. `run_post_build`'s `RunCommand` arm
/// (and `PostBuildOutcome::skipped_missing_tools`) depend on `Ok(false)` here to tell the
/// two apart, so the primitive that ultimately makes the skip observable must be pinned
/// down on its own. ~keep
#[test]
fn run_run_command_reports_false_when_the_tool_is_not_on_path() {
    let _guard = SkipCommandsGuard::set("");
    let dir = std::env::temp_dir();
    let result = run_run_command(
        "alef-definitely-not-a-real-binary-xyz123",
        &[],
        &dir,
        "sample",
        super::RUN_COMMAND_TIMEOUT,
    );
    assert!(
        matches!(result, Ok(false)),
        "a missing tool must be reported as skipped, not silently equivalent to success: {result:?}"
    );
}

/// `run_run_command` used to enforce a bare module constant (`RUN_COMMAND_TIMEOUT`, 1800s) with
/// no way to shorten or lengthen it per call -- this proves the `timeout` parameter this fix
/// adds is actually honored by the kill loop, not merely accepted and ignored. A `sleep 3` run
/// under a 1-second ceiling must be killed at ~1s, not run to completion and not wait for the
/// unrelated 1800s default still in force elsewhere in this file. ~keep
#[test]
fn run_run_command_is_killed_at_a_shorter_than_default_timeout() {
    let _guard = SkipCommandsGuard::set("");
    let dir = std::env::temp_dir();
    let started = std::time::Instant::now();

    let result = run_run_command("sleep", &["3"], &dir, "sample", std::time::Duration::from_secs(1));
    let elapsed = started.elapsed();

    let error = result.expect_err("a sleep 3 under a 1s ceiling must time out");
    assert!(
        error.to_string().contains("exceeded 1s timeout"),
        "error should name the configured 1s ceiling, not the 1800s default: {error:#}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "must be killed at the configured 1s ceiling rather than running to completion: {elapsed:?}"
    );
}

/// End-to-end proof that `[build_commands.<lang>].timeout_seconds` in `alef.toml` actually
/// reaches `run_run_command`'s enforcement point -- alef #364's real defect was not that the
/// timeout fired too early, it was that there was no config surface reaching it at all. Asserting
/// the config merely parses would not catch that: this drives the same `run_post_build` entry
/// point `alef generate`/`alef all` use, with a post-build `RunCommand` step that legitimately
/// runs longer than the configured ceiling, and checks the kill fires at THAT ceiling. ~keep
#[test]
fn configured_build_command_timeout_reaches_the_post_build_run_command_step() {
    let _guard = SkipCommandsGuard::set("");
    let directory = tempfile::tempdir().expect("temporary project");

    let mut config = crate::core::config::ResolvedCrateConfig::default();
    config.build_commands.insert(
        Language::Swift.to_string(),
        BuildCommandConfig {
            precondition: None,
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: None,
            build_release: None,
            timeout_seconds: Some(1),
        },
    );

    let build_config = crate::core::backend::BuildConfig {
        tool: "swift",
        crate_suffix: "-swift",
        build_dep: crate::core::backend::BuildDependency::None,
        post_build: vec![crate::core::backend::PostBuildStep::RunCommand {
            cmd: "sleep",
            args: vec!["3"],
        }],
    };

    let started = std::time::Instant::now();
    let result = run_post_build(Language::Swift, &build_config, &config, directory.path());
    let elapsed = started.elapsed();

    let error = result.expect_err("a sleep 3 post-build step under a configured 1s ceiling must fail");
    let message = format!("{error:#}");
    assert!(message.contains("exceeded 1s timeout"), "got: {message}");
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "the configured 1s ceiling must fire before the sleep completes or the 1800s default \
         would: {elapsed:?}"
    );
}
