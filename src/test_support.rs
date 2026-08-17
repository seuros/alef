//! Test-only support shared across the whole crate.
//!
//! `cargo test` runs every `#[test]` as a thread inside one process, so any state a test mutates
//! through a process-global API -- like `std::env::set_current_dir` -- is shared mutable state
//! across every other test in the binary, not just the tests in the same module. Before this
//! module existed, four separate `CWD_LOCK` statics lived in `cli::cache`,
//! `cli::breaking_changes`, `cli::pipeline::version_tests`, and
//! `cli::pipeline::generate::generation` (plus an unguarded fifth lock local to
//! `bin_cli::all_commands_tests`), each correctly serializing the tests in its own module but
//! doing nothing to serialize against the other four -- so two cwd-mutating tests from different
//! modules could still run concurrently and race. [`CWD_LOCK`] is the one lock every cwd-mutating
//! test in this crate now shares. ~keep

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The single lock serializing every test in this crate that mutates the process-global current
/// directory. See the module docs for why one shared lock is required rather than one per module.
pub(crate) static CWD_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that enters `dir` as the process current directory for its lifetime and restores
/// the original directory on drop -- including when the guarded scope panics, since `Drop` still
/// runs while a panic unwinds. Holds [`CWD_LOCK`] for its entire lifetime, so at most one
/// `CwdGuard` is ever live across the whole crate at a time.
///
/// A poisoned lock (an earlier guard's scope panicked while holding it) is still acquired: one
/// panicking test must not cascade into every other cwd-mutating test failing on a poisoned
/// mutex, and the poison carries no invalidated data here -- the guard that poisoned the lock had
/// already restored its own original directory via `Drop` before the panic finished unwinding
/// through it.
pub(crate) struct CwdGuard {
    _lock: MutexGuard<'static, ()>,
    original: PathBuf,
}

impl CwdGuard {
    /// Locks [`CWD_LOCK`] and enters `dir` as the process current directory, returning a guard
    /// that restores the original directory when dropped.
    pub(crate) fn enter(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original = std::env::current_dir().expect("read current directory");
        std::env::set_current_dir(dir).expect("enter directory");
        Self { _lock: lock, original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}
