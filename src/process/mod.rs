//! Subprocess-tree lifecycle for every alef path that runs a child under a deadline.
//!
//! Two facts drive everything here, and they were learned twice -- once by the snippet validators,
//! then again by the `setup`/`build` pipeline, because the second path carried its own copy of the
//! naive shape. A `sh -c` child is a *tree*: `Child::kill` signals the shell alone, so the
//! `gradlew` it started, and the Gradle daemon under that, survive their own deadline and reparent
//! to PID 1. Killing the tree means killing a process group, which means spawning into one. And a
//! child in its own process group is no longer in the terminal's foreground group, so it never
//! receives the `SIGINT` Ctrl-C delivers -- alef has to forward that itself.
//!
//! The two halves are inseparable: [`configure_process_group`] without [`termination::track`]
//! trades an orphaned tree on timeout for an orphaned tree on Ctrl-C. Call them together. ~keep

pub(crate) mod capture;
#[cfg(windows)]
pub(crate) mod job;
pub(crate) mod termination;
pub(crate) mod timed;

/// Puts `command`'s child in a new process group of its own, so its whole tree can be addressed
/// by one signal.
///
/// Only meaningful alongside [`termination::track`] -- see the module docs.
#[cfg(unix)]
pub(crate) fn configure_process_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows has nothing to configure before the spawn. Its tree is addressed by a job object,
/// which can only be joined by a process that already exists, so the whole Windows half of this
/// happens in [`termination::track`] instead -- and a Windows child stays in alef's own console
/// group, so it receives Ctrl-C by delivery and needs no forwarding. ~keep
#[cfg(not(unix))]
pub(crate) fn configure_process_group(_command: &mut std::process::Command) {}

/// Kills `child` and every descendant it started.
///
/// `tracked` is the registry slot [`termination::track`] handed back for this same child. It is
/// not decoration on Windows: the job object that makes the kill tree-wide lives in it, and
/// passing the wrong one -- or none -- silently degrades this to killing the direct child.
///
/// Falls back to signalling the child alone when the tree kill fails, which is the best that can
/// be done for a child that was not spawned through [`configure_process_group`] and
/// [`termination::track`].
#[cfg(unix)]
pub(crate) fn kill_process_tree(child: &mut std::process::Child, _tracked: &termination::TrackedProcessGroup) {
    let process_group = format!("-{}", child.id());
    let killed_group = std::process::Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .status()
        .is_ok_and(|status| status.success());
    if !killed_group {
        let _ = child.kill();
    }
}

#[cfg(windows)]
pub(crate) fn kill_process_tree(child: &mut std::process::Child, tracked: &termination::TrackedProcessGroup) {
    if !tracked.terminate_tree() {
        let _ = child.kill();
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn kill_process_tree(child: &mut std::process::Child, _tracked: &termination::TrackedProcessGroup) {
    let _ = child.kill();
}

/// The first gap between `try_wait` polls. A fixed 50ms interval charged every subprocess about
/// 25ms of pure sleep on average — invisible for one `cargo check`, tens of seconds across a run
/// with thousands of snippets, because most snippet toolchain invocations finish in single-digit
/// milliseconds. ~keep
pub(crate) const INITIAL_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// The ceiling the backoff grows to, so a genuinely long compile still costs at most one wakeup
/// per 50ms rather than a thousand. ~keep
const MAX_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Doubles a poll interval up to [`MAX_WAIT_POLL_INTERVAL`].
pub(crate) fn next_poll_interval(current: std::time::Duration) -> std::time::Duration {
    current
        .checked_mul(2)
        .unwrap_or(MAX_WAIT_POLL_INTERVAL)
        .min(MAX_WAIT_POLL_INTERVAL)
}

/// How long a child may be observed continuously in the kernel's stopped state (macOS/Linux
/// `STAT=T`) before a bounded wait gives up on it early, rather than waiting out the rest of its
/// timeout budget.
///
/// STOPPED is not slow. A stopped process makes no progress until something sends it `SIGCONT`,
/// and nothing in alef's own child-management ever does -- a job-control stop (a child put in a
/// background process group that touches its controlling terminal, most often by reading stdin)
/// is a dead end, not a delay. Waiting out an ordinary timeout on it only postpones the same
/// outcome; this bound is what turns "eventually notices, after however long the caller's timeout
/// happens to be" into "notices within a couple of seconds regardless of the timeout." ~keep
pub(crate) const STOPPED_PROCESS_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// The minimum gap between successive process-state probes inside a bounded wait.
///
/// Coarse enough that a multi-minute build does not spend its wall clock re-checking `ps` on every
/// poll tick; fine enough that [`STOPPED_PROCESS_GRACE`] still resolves to a small, predictable
/// number of checks. ~keep
pub(crate) const STOPPED_PROCESS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// How long [`kill_process_tree`]'s caller may wait for the kill to actually be reaped before
/// giving up on the wait rather than the process.
///
/// A `SIGKILL` delivered to a process group is not a promise the target is gone before the next
/// instruction runs -- the reap is a separate, later event. Bounding it is what keeps a kill that
/// silently failed to reach its target (wrong group, a permission error swallowed by the `kill`
/// child command, a target that reparented out of the group) from turning "the process is now
/// being torn down" into a second, unconditional hang immediately behind the first. ~keep
pub(crate) const KILL_REAP_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether the process named by `pid` is currently stopped (kernel state `T`/`t`) rather than
/// running, sleeping, or already gone.
///
/// Shells out to `ps` instead of reading `/proc` because the state this exists to catch was first
/// observed on macOS, which has no `/proc`; `ps -o state=` reports the same single-letter state
/// column on both platforms alef supports. Any failure to ask -- the pid raced to exit, the host
/// has no `ps` -- reads as "not stopped": the ordinary timeout this augments still catches a child
/// in any other state, so a probe failure only forfeits the early exit, not correctness. ~keep
#[cfg(unix)]
pub(crate) fn is_process_stopped(pid: u32) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-o", "state=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).trim().starts_with(['T', 't'])
}

#[cfg(not(unix))]
pub(crate) fn is_process_stopped(_pid: u32) -> bool {
    false
}

pub(crate) trait WaitTimeout {
    /// Waits up to `timeout` for the child to exit, returning `Ok(None)` when it is still running.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be waited on.
    fn wait_timeout(&mut self, timeout: std::time::Duration) -> std::io::Result<Option<std::process::ExitStatus>>;
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
mod tests {
    use std::time::Duration;

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
}
