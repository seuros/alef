//! The persistent, per-session directories a toolchain keeps its compiled artifacts in, and the
//! bounded retention that stops them accumulating without limit.
//!
//! `.alef/snippets/sessions/` has been swept against the run's live set since
//! `purge::purge_stale_session_scratch` landed; `.alef/snippets/cache/` never was, and
//! nothing else removed from it either. Every key that fell out of use kept its whole cargo target
//! directory forever -- one consumer reached 19.9 GiB across five directories. This module is the
//! missing counterpart, deliberately shaped after the scratch sweep it sits beside. ~keep

use super::{ResolvedSession, ValidationSession, fingerprint::session_toolchain_key};
use crate::snippets::error::{Error, Result};
use crate::snippets::scratch::ABANDONED_GRACE_SECS;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The in-tree root every toolchain cache directory nests under, relative to a session's
/// `working_directory`.
const TOOLCHAIN_CACHE_ROOT: &str = ".alef/snippets/cache";

/// Written into a key's directory once per run that resolves that key, so "when was this cache
/// last used" is an explicit fact rather than an inference from whatever a toolchain happened to
/// write. A run whose snippets all hit the verdict cache launches no compiler at all, so without
/// this its directory would look untouched and be first in line for reclamation. ~keep
const USE_STAMP: &str = ".alef-last-used";

/// How deep [`last_used`] looks for a recent write. Depth 3 reaches `<key>/cargo-target/debug/`
/// and `<key>/cargo-target/.rustc_info.json` -- the latter rewritten by every single `cargo`
/// invocation -- which is what makes an actively compiling directory look recent to a concurrent
/// run's sweep. Deeper would start listing `.fingerprint/`'s thousands of entries for no further
/// signal. ~keep
const RECENCY_DEPTH: usize = 3;

/// How many *non-live* toolchain cache generations survive a run.
///
/// A generation only falls out of the live set when the session's configuration changes, so this
/// is not a per-run allowance -- it is how many previous configurations keep their compiled
/// artifacts. One is enough to make flipping a feature flag back and forth cheap while bounding a
/// consumer at two generations per session rather than one per run. ~keep
pub const DEFAULT_TOOLCHAIN_CACHE_GENERATIONS: usize = 1;

pub(super) fn toolchain_cache_root(working_directory: &Path) -> PathBuf {
    working_directory.join(TOOLCHAIN_CACHE_ROOT)
}

/// The persistent, per-session directories a toolchain keeps its compiled artifacts in. All are
/// keyed by the session's toolchain key and survive across runs -- that reuse is the entire
/// point. ~keep
pub(super) struct ToolchainCaches {
    root: PathBuf,
    pub(super) go_build: PathBuf,
    pub(super) zig_global: PathBuf,
    pub(super) cargo_target: PathBuf,
}

impl ToolchainCaches {
    pub(super) fn directories(&self) -> [&Path; 3] {
        [&self.go_build, &self.zig_global, &self.cargo_target]
    }

    /// Records that this run used these caches, so a later sweep orders them by use rather than by
    /// whatever a toolchain last wrote. Tolerated on failure for the same reason the sweeps are:
    /// losing the stamp costs retention accuracy, while failing preparation over it would black
    /// out the language the cache exists to keep fast. ~keep
    pub(super) fn mark_used(&self) {
        let stamp = self.root.join(USE_STAMP);
        if let Err(error) = std::fs::File::create(&stamp) {
            tracing::debug!(stamp = %stamp.display(), error = %error, "could not stamp snippet toolchain cache use");
        }
    }
}

pub(super) fn cache_directories(session: &ValidationSession) -> ToolchainCaches {
    let root = toolchain_cache_root(&session.working_directory).join(session_toolchain_key(session));
    ToolchainCaches {
        go_build: root.join("go-build"),
        zig_global: root.join("zig-global"),
        cargo_target: root.join("cargo-target"),
        root,
    }
}

/// Sweeps the toolchain cache root of every working directory this run touches, keeping every key
/// the run is about to use plus the `generations` most recently used of the rest.
///
/// Unlike the scratch sweep next door, this one keeps *every* resolved session's key including
/// Java's: Java's per-snippet scratch lives outside the working directory, but its toolchain cache
/// does not, so omitting it here would reclaim a live cache.
///
/// A sweep failure is logged and tolerated rather than propagated: losing the sweep costs disk,
/// while failing preparation over it would black out exactly the language the cache exists to keep
/// fast. ~keep
pub(super) fn purge_stale_toolchain_caches(resolved: &[ResolvedSession<'_>], timeout_secs: u64, generations: usize) {
    let Some(cutoff) = reclaim_cutoff(timeout_secs) else {
        return;
    };
    let mut live: BTreeMap<&Path, BTreeSet<String>> = BTreeMap::new();
    for (_, spec, session) in resolved {
        live.entry(spec.working_directory.as_path())
            .or_default()
            .insert(session_toolchain_key(session));
    }
    for (working_directory, keys) in live {
        if let Err(error) = purge_toolchain_cache_root(working_directory, &keys, cutoff, generations) {
            tracing::warn!(
                working_directory = %working_directory.display(),
                error = %error,
                "could not purge stale snippet toolchain caches"
            );
        }
    }
}

/// The instant a directory must not have been touched since, to be considered reclaimable.
///
/// Mirrors `purge::cleanup_legacy_scratch_directories`: one run's own per-snippet timeout
/// bounds how long a toolchain invocation can go without writing, and [`ABANDONED_GRACE_SECS`] is
/// the same margin the scratch sweep uses to tell a concurrently running process's work in
/// progress apart from a crashed run's leftovers. Two alef processes can legitimately share one
/// working directory, and reclaiming a directory a live `cargo` is writing into fails that build
/// outright rather than merely costing it a rebuild. ~keep
fn reclaim_cutoff(timeout_secs: u64) -> Option<SystemTime> {
    SystemTime::now().checked_sub(Duration::from_secs(timeout_secs.saturating_add(ABANDONED_GRACE_SECS)))
}

fn purge_toolchain_cache_root(
    working_directory: &Path,
    live: &BTreeSet<String>,
    cutoff: SystemTime,
    generations: usize,
) -> Result<()> {
    let root = toolchain_cache_root(working_directory);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::Other(format!(
                "reading snippet toolchain cache root {}: {error}",
                root.display()
            )));
        }
    };
    let mut reclaimable = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::Other(format!(
                "reading an entry in snippet toolchain cache root {}: {error}",
                root.display()
            ))
        })?;
        let path = entry.path();
        if !entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
            remove(std::fs::remove_file(&path), &path)?;
            continue;
        }
        if entry.file_name().to_str().is_some_and(|name| live.contains(name)) {
            continue;
        }
        reclaimable.push((last_used(&path), path));
    }
    reclaimable.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    for (used, path) in reclaimable.into_iter().skip(generations) {
        if used > cutoff {
            continue;
        }
        remove(std::fs::remove_dir_all(&path), &path)?;
    }
    Ok(())
}

/// The most recent modification anywhere in the shallow top of `directory`, including its
/// [`USE_STAMP`].
fn last_used(directory: &Path) -> SystemTime {
    walkdir::WalkDir::new(directory)
        .max_depth(RECENCY_DEPTH)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter_map(|metadata| metadata.modified().ok())
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn remove(outcome: std::io::Result<()>, path: &Path) -> Result<()> {
    match outcome {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Other(format!(
            "removing stale snippet toolchain cache {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests;
