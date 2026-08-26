//! One `Mutex` per resolved snippet-validation session, keyed by *fingerprint* rather than by the
//! config session name it was reached through. Split out of `runner` under the repo's file-size
//! cap.
//!
//! Multiple session names can resolve to the same fingerprint: same `cwd`, manifest, env,
//! features (see `alef.toml`'s own `[workspace.docs.snippets.sessions.typescript]` comment on
//! aliasing `typescript` and `node` to the same package), which is also the same physical
//! workspace directory a batch validator writes scratch files into. Keying the lock by name
//! instead handed each alias its own `Mutex`, so two batch groups that both believed they held
//! "the" session lock wrote into the same `snippet_batch_N.ts` files concurrently. ~keep

use std::collections::HashMap;
use std::sync::Mutex;

/// Deduplicates on fingerprint, so two config names sharing one fingerprint end up pointing at the
/// identical `Mutex`.
pub(super) fn session_locks_by_fingerprint<'a>(
    sessions: impl Iterator<Item = &'a crate::snippets::session::ValidationSession>,
) -> HashMap<String, Mutex<()>> {
    let mut locks = HashMap::new();
    for session in sessions {
        locks.entry(session.fingerprint.clone()).or_insert_with(|| Mutex::new(()));
    }
    locks
}

/// Resolves the `Mutex` a snippet's session actually shares with every other session aliased to
/// the same fingerprint. Looking this up by the session's *name* instead was the bug this module
/// exists to fix.
pub(super) fn session_lock_for<'a>(
    session: Option<&crate::snippets::session::ValidationSession>,
    session_locks: &'a HashMap<String, Mutex<()>>,
) -> Option<&'a Mutex<()>> {
    session.and_then(|session| session_locks.get(session.fingerprint.as_str()))
}
