//! The one place that knows the order of a deadline-bounded subprocess's lifecycle.
//!
//! Spawn into a group, register that group for signal forwarding, wait, and on expiry kill the
//! group rather than the child. The order is not interchangeable: [`configure_process_group`] has
//! to run before `spawn` because it configures the `Command`, and [`termination::track`] has to
//! run after it because it needs the pid. Getting either wrong yields a tree that outlives its own
//! deadline, or one that Ctrl-C can no longer reach -- neither of which is visible at the call
//! site. Callers get [`GroupChild`] instead of the three calls in the right order. ~keep

use crate::process::termination::TrackedProcessGroup;
use crate::process::{WaitTimeout as _, configure_process_group, kill_process_tree, termination};

/// What a bounded wait found.
pub(crate) enum Deadline {
    /// The child exited on its own, with this status.
    Exited(std::process::ExitStatus),
    /// The budget elapsed first. The whole process group has already been killed and reaped.
    Expired,
}

/// A child that leads its own process group, held together with the registry slot that forwards
/// this process's termination signals to that group.
///
/// The slot is released when the value is dropped, so a group that is no longer being waited on
/// stops being swept by the Ctrl-C handler.
pub(crate) struct GroupChild {
    child: std::process::Child,
    _tracked: TrackedProcessGroup,
}

impl GroupChild {
    /// Spawns `command` as the leader of a new process group and registers that group for
    /// termination forwarding.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be spawned.
    pub(crate) fn spawn(command: &mut std::process::Command) -> std::io::Result<Self> {
        configure_process_group(command);
        let child = command.spawn()?;
        let tracked = termination::track(&child);
        Ok(Self {
            child,
            _tracked: tracked,
        })
    }

    pub(crate) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// Kills the child and every descendant it started, then reaps the child.
    ///
    /// Safe to call on a child that has already exited: the group kill fails, the fallback
    /// `Child::kill` fails, and the reap returns the status already collected.
    pub(crate) fn kill_tree(&mut self) {
        kill_process_tree(&mut self.child);
        let _ = self.child.wait();
    }

    /// Waits up to `budget` for the child to exit, tearing its whole tree down when the budget
    /// runs out first.
    ///
    /// `command` names the command in the timeout warning and is only formatted when the deadline
    /// is actually missed.
    ///
    /// # Errors
    ///
    /// Returns an error when the child cannot be waited on. The tree is killed in that case too:
    /// a wait that failed leaves nothing else able to reap it.
    pub(crate) fn wait_within(
        &mut self,
        budget: std::time::Duration,
        command: &impl std::fmt::Debug,
    ) -> std::io::Result<Deadline> {
        match self.child.wait_timeout(budget) {
            Ok(Some(status)) => Ok(Deadline::Exited(status)),
            Ok(None) => {
                tracing::warn!(
                    command = ?command,
                    budget_seconds = budget.as_secs(),
                    "command exceeded its timeout; killing its process group"
                );
                self.kill_tree();
                Ok(Deadline::Expired)
            }
            Err(error) => {
                self.kill_tree();
                Err(error)
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{Deadline, GroupChild};
    use std::time::Duration;

    const SETTLE_POLL: Duration = Duration::from_millis(20);
    const SETTLE_LIMIT: Duration = Duration::from_secs(5);

    fn is_alive(pid: i32) -> bool {
        // SAFETY: signal 0 performs error checking only and sends nothing.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_until_gone(pid: i32) -> bool {
        let deadline = std::time::Instant::now() + SETTLE_LIMIT;
        while std::time::Instant::now() < deadline {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(SETTLE_POLL);
        }
        !is_alive(pid)
    }

    /// Spawns `sh -c 'sleep 60 & echo $! > marker; sleep 60'` through [`GroupChild`] and returns
    /// it together with its grandchild's pid, once the grandchild is confirmed running.
    fn spawn_with_grandchild(directory: &std::path::Path) -> (GroupChild, i32) {
        let marker = directory.join("grandchild.pid");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", &format!("sleep 60 & echo $! > {}; sleep 60", marker.display())]);
        let child = GroupChild::spawn(&mut command).expect("spawn the process group");

        let deadline = std::time::Instant::now() + SETTLE_LIMIT;
        let grandchild = loop {
            assert!(
                std::time::Instant::now() < deadline,
                "grandchild never announced itself"
            );
            if let Ok(contents) = std::fs::read_to_string(&marker)
                && let Ok(pid) = contents.trim().parse::<i32>()
                && is_alive(pid)
            {
                break pid;
            }
            std::thread::sleep(SETTLE_POLL);
        };
        (child, grandchild)
    }

    /// The defect this module exists to close. `Child::kill` signals the `sh` wrapper alone, so
    /// the grandchild it started outlives the deadline and reparents to PID 1 -- a Gradle daemon,
    /// in the incident that prompted this. Asserting that the kill branch was entered would prove
    /// nothing: the orphaned tree was produced by code that did enter its kill branch. ~keep
    #[test]
    fn an_expired_deadline_kills_the_grandchild_too() {
        let directory = tempfile::tempdir().expect("scratch directory");
        let (mut child, grandchild) = spawn_with_grandchild(directory.path());

        let outcome = child
            .wait_within(Duration::from_millis(200), &"sleep 60 & sleep 60")
            .expect("waiting on a live child is not an error");

        assert!(matches!(outcome, Deadline::Expired));
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} survived its parent's deadline"
        );
    }

    /// The sabotage check for the test above: were the wait to report expiry for a child that had
    /// exited on its own, or the kill to run unconditionally, that test would pass for the wrong
    /// reason. The exact status is asserted because a bounded wait that loses the child's exit
    /// code turns every timed command into a silent success. ~keep
    #[test]
    fn a_child_that_beats_its_deadline_reports_its_own_exit_status() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "exit 7"]);
        let mut child = GroupChild::spawn(&mut command).expect("spawn the process group");

        let outcome = child
            .wait_within(Duration::from_secs(30), &"exit 7")
            .expect("waiting on a live child is not an error");

        match outcome {
            Deadline::Exited(status) => assert_eq!(status.code(), Some(7)),
            Deadline::Expired => panic!("a command that exits immediately must not report expiry"),
        }
    }
}
