//! What the `setup`/`build` pipeline's deadline is worth end to end.
//!
//! [`crate::process::timed`] proves the group lifecycle in isolation. These prove the pipeline
//! actually uses it: a helper that kept the old `Child::kill` would pass every test in that module
//! and still orphan a Gradle daemon here. ~keep

use std::time::{Duration, Instant};

const SETTLE_POLL: Duration = Duration::from_millis(20);
const SETTLE_LIMIT: Duration = Duration::from_secs(10);

/// The deadline the two timeout tests run their child under. Long enough that reading the
/// grandchild's announcement out of the marker file is never a race against the kill, short enough
/// that the two tests together cost a few seconds. ~keep
const PROBE_BUDGET_SECS: u64 = 3;

/// Names the file the probe below announces its child's process group in. Its presence is also
/// what tells the probe it is running as a probe rather than as an ordinary ignored test.
const ORPHAN_PROBE_MARKER: &str = "ALEF_PIPELINE_ORPHAN_PROBE";
const ORPHAN_PROBE_NAME: &str = "cli::pipeline::helpers::timeout_tests::pipeline_orphan_probe_child";

fn is_alive(pid: i32) -> bool {
    // SAFETY: signal 0 performs error checking only and sends nothing.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_until_gone(pid: i32) -> bool {
    let deadline = Instant::now() + SETTLE_LIMIT;
    while Instant::now() < deadline {
        if !is_alive(pid) {
            return true;
        }
        std::thread::sleep(SETTLE_POLL);
    }
    !is_alive(pid)
}

/// Blocks until `marker` holds a pid and returns it.
///
/// Liveness is deliberately not part of the wait. The pid is asserted *gone* later, and a loop
/// that also required it to be alive would race the very kill under test: on a loaded machine the
/// tree can be torn down before the marker is read, and the test would fail for succeeding. ~keep
fn announced_pid(marker: &std::path::Path) -> i32 {
    let deadline = Instant::now() + SETTLE_LIMIT;
    loop {
        assert!(Instant::now() < deadline, "no pid was ever announced in the marker file");
        if let Ok(contents) = std::fs::read_to_string(marker)
            && let Ok(pid) = contents.trim().parse::<i32>()
            && pid > 0
        {
            return pid;
        }
        std::thread::sleep(SETTLE_POLL);
    }
}

/// A shell script that backgrounds a long sleep, announces that sleep's pid in `marker`, and then
/// sleeps well past any deadline a test will give it.
fn script_leaking_a_grandchild(marker: &std::path::Path) -> String {
    format!("sleep 300 & echo $! > {}; sleep 300", marker.display())
}

fn assert_timed_out(outcome: &anyhow::Error) {
    let message = outcome.to_string();
    assert!(
        message.contains(&format!("timed out after {PROBE_BUDGET_SECS}s")),
        "unexpected error: {message}"
    );
}

/// The defect. `alef setup` runs its install commands through this helper, which used to kill the
/// `sh` wrapper and leave the tree under it -- `sh -> gradlew -> daemon` -- running past its own
/// deadline and reparented to PID 1. The grandchild's pid is asserted to stop existing; asserting
/// that the timeout branch was entered would prove nothing, because the orphaned tree was produced
/// by code that did enter that branch. ~keep
#[test]
fn a_streamed_command_that_times_out_kills_its_grandchild_too() {
    let directory = tempfile::tempdir().expect("scratch directory");
    let marker = directory.path().join("grandchild.pid");
    let script = script_leaking_a_grandchild(&marker);

    let grandchild = std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            super::run_command_streamed_with_cwd_and_timeout(&script, Some("probe"), Some(PROBE_BUDGET_SECS), None)
        });
        let grandchild = announced_pid(&marker);
        let outcome = running.join().expect("the streamed command returns");
        assert_timed_out(&outcome.expect_err("a 300s script under a short deadline must fail"));
        grandchild
    });

    assert!(
        wait_until_gone(grandchild),
        "grandchild {grandchild} survived the deadline that killed its parent"
    );
}

/// The same property for the captured variant, which `setup` uses for its `before` hooks. It reads
/// the child's pipes rather than pumping them into alef's stderr, so it reaches the deadline down
/// a different path and has to be measured separately. ~keep
#[test]
fn a_captured_command_that_times_out_kills_its_grandchild_too() {
    let directory = tempfile::tempdir().expect("scratch directory");
    let marker = directory.path().join("grandchild.pid");
    let script = script_leaking_a_grandchild(&marker);

    let grandchild = std::thread::scope(|scope| {
        let running = scope.spawn(|| super::run_command_captured_with_timeout(&script, Some(PROBE_BUDGET_SECS)));
        let grandchild = announced_pid(&marker);
        let outcome = running.join().expect("the captured command returns");
        assert_timed_out(&outcome.expect_err("a 300s script under a short deadline must fail"));
        grandchild
    });

    assert!(
        wait_until_gone(grandchild),
        "grandchild {grandchild} survived the deadline that killed its parent"
    );
}

/// The bounded-drain half, which is the one that hides. `sh -c 'sleep 300 & exit 0'` exits at once
/// but hands its stdout to a descendant that does not, so the old code satisfied its deadline and
/// then blocked forever reading the pipes: a bounded wait followed by an unbounded drain is still
/// unbounded. The command's own budget is 120s, far longer than this may take, so anything beyond
/// the drain grace is that shape coming back rather than the deadline doing the work. Generous
/// headroom because this proves termination, not the exact grace. ~keep
#[test]
fn a_leaked_descendant_holding_the_pipes_cannot_outlast_the_drain_grace() {
    let grace = crate::process::capture::OUTPUT_DRAIN_GRACE;
    let started = Instant::now();

    let outcome = super::run_command_captured_with_timeout("sleep 300 & exit 0", Some(120));
    let elapsed = started.elapsed();

    assert!(outcome.is_ok(), "the command itself exits zero: {outcome:?}");
    assert!(
        elapsed < grace * 3,
        "draining took {elapsed:?}, which is not bounded by the {grace:?} grace"
    );
}

/// Not a test: the child half of [`a_signalled_alef_does_not_orphan_a_pipeline_child_tree`]. Runs
/// a pipeline command whose child announces its own process group and then outlives any signal the
/// parent will send. Inert unless the environment names a marker file, so an ordinary `--ignored`
/// run does nothing. ~keep
#[test]
#[ignore = "spawned as a subprocess by a_signalled_alef_does_not_orphan_a_pipeline_child_tree"]
fn pipeline_orphan_probe_child() {
    let Ok(marker) = std::env::var(ORPHAN_PROBE_MARKER) else {
        return;
    };
    let script = format!("echo $$ > {marker}; sleep 120");
    let _ = super::run_command_streamed_with_cwd_and_timeout(&script, None, Some(120), None);
}

/// The regression the group spawn risks, and why the group half must not ship without the signal
/// half. Putting the child in its own process group takes it out of the terminal's foreground
/// group, so `SIGINT` -- what Ctrl-C delivers -- no longer reaches it by delivery; only
/// [`crate::process::termination`]'s forwarding can. A real alef process is spawned, given a child
/// that outlives everything, and signalled. Asserting the child is gone *after the parent is* is
/// the whole point: without forwarding, `alef setup` regresses from stoppable to not, and this is
/// the only thing that would say so. ~keep
#[test]
fn a_signalled_alef_does_not_orphan_a_pipeline_child_tree() {
    let directory = tempfile::tempdir().expect("scratch directory");
    let marker = directory.path().join("group.pid");
    let mut probe = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", ORPHAN_PROBE_NAME, "--ignored", "--test-threads=1"])
        .env(ORPHAN_PROBE_MARKER, &marker)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the probe");

    let orphan = announced_pid(&marker);
    assert!(is_alive(orphan), "the probe's child must be running before the signal");

    // SAFETY: signalling one pid this test spawned and owns.
    let signalled = unsafe { libc::kill(probe.id().cast_signed(), libc::SIGINT) };
    assert_eq!(signalled, 0, "signalling the probe");
    probe.wait().expect("the probe exits");

    let survived = !wait_until_gone(orphan);
    if survived {
        // SAFETY: a negative pid addresses the process group the probe created.
        unsafe {
            libc::kill(-orphan, libc::SIGKILL);
        }
    }
    assert!(
        !survived,
        "child group {orphan} outlived the alef process that spawned it -- the tree was orphaned"
    );
}
