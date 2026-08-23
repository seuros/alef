use super::*;
use crate::test_support::SkipCommandsGuard;

#[test]
fn run_run_command_succeeds_for_echo() {
    let _guard = SkipCommandsGuard::set("");
    let dir = std::env::temp_dir();
    let result = run_run_command("echo", &["alef-runcommand-ok"], &dir, "sample");
    assert!(
        matches!(result, Ok(true)),
        "echo should run and report Ok(true): {result:?}"
    );
}

#[test]
fn run_run_command_fails_for_false() {
    let _guard = SkipCommandsGuard::set("");
    let dir = std::env::temp_dir();
    let result = run_run_command("false", &[], &dir, "sample");
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
        let skipped = run_run_command("false", &[], &dir, "sample");
        assert!(
            matches!(skipped, Ok(false)),
            "listed command must return Ok(false) without spawning: {skipped:?}"
        );
    }

    let _guard = SkipCommandsGuard::set("something-else");
    let honored = run_run_command("false", &[], &dir, "sample");
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
    let result = run_run_command("alef-definitely-not-a-real-binary-xyz123", &[], &dir, "sample");
    assert!(
        matches!(result, Ok(false)),
        "a missing tool must be reported as skipped, not silently equivalent to success: {result:?}"
    );
}
