//! Scratch-directory sweeping for snippet validation sessions.
//!
//! Split out of `session` under the repo's file-size cap. Every function here is a *tolerated*
//! cleanup: a sweep failure is logged, never propagated, because losing the sweep costs
//! cleanliness while failing preparation over it would black out exactly the language the sweep
//! exists to keep running. ~keep

use super::{ResolvedSession, SESSION_SCRATCH_ROOT, SessionSpec, keeps_scratch_outside_working_directory};
use crate::snippets::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Sweeps the in-tree session scratch root of every working directory this run touches, keeping
/// only the fingerprints this run is actually about to use.
///
/// Two distinct leftovers accumulate there and both black out a whole language:
///
/// - A *stale fingerprint's whole directory*. The fingerprint changes whenever the working tree
///   does, so every day's run mints a new directory and nothing ever removed the previous ones. In
///   a Java package this is fatal rather than untidy: alef's Java backend points Maven's
///   `<sourceDirectory>` at `${project.basedir}`, so a `before` hook of `mvn package` compiles
///   every `.java` under it — four days' worth of leftover `Example.java` at once, which `javac`
///   rejects with `duplicate class: Example`, failing session preparation and stamping every
///   snippet of that language `SnippetStatus::Error`. `JavaValidator` has written its scratch
///   outside `working_directory` since the `external_workspace_directory` fix, so java's live set
///   here is empty and every directory it finds is a pre-fix leftover — but the accumulation is
///   not java-specific, so neither is the sweep.
/// - A *stray top-level file inside a live fingerprint's directory*, left by a previous run's
///   per-snippet validate call (`Program.cs`, `snippet.ts`, ...). That directory is deliberately
///   reused across runs so compiled-artifact caches in its subdirectories survive, so it is kept
///   and only its direct file children are removed — never a subdirectory (`target/`, `.nuget/`,
///   `dist/`, ...), never recursing.
///
/// A sweep failure is logged and tolerated rather than propagated: losing the sweep costs
/// cleanliness, while failing preparation over it would black out exactly the language the sweep
/// exists to keep running. ~keep
pub(super) fn purge_stale_session_scratch(resolved: &[ResolvedSession<'_>]) {
    let mut live: BTreeMap<&Path, BTreeSet<&str>> = BTreeMap::new();
    for (_, spec, session) in resolved {
        let fingerprints = live.entry(spec.working_directory.as_path()).or_default();
        if !keeps_scratch_outside_working_directory(spec.language) {
            fingerprints.insert(session.fingerprint.as_str());
        }
    }
    for (working_directory, fingerprints) in live {
        if let Err(error) = purge_session_scratch_root(working_directory, &fingerprints) {
            tracing::warn!(
                working_directory = %working_directory.display(),
                error = %error,
                "could not purge stale snippet session scratch"
            );
        }
    }
}

fn purge_session_scratch_root(working_directory: &Path, live_fingerprints: &BTreeSet<&str>) -> Result<()> {
    let root = working_directory.join(SESSION_SCRATCH_ROOT);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::Other(format!(
                "reading snippet session scratch root {}: {error}",
                root.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::Other(format!(
                "reading an entry in snippet session scratch root {}: {error}",
                root.display()
            ))
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        let is_live = name.to_str().is_some_and(|name| live_fingerprints.contains(name));
        if !is_directory {
            remove_scratch(std::fs::remove_file(&path), &path)?;
        } else if is_live {
            purge_stale_workspace_scratch_files(&path)?;
        } else {
            remove_scratch(std::fs::remove_dir_all(&path), &path)?;
        }
    }
    Ok(())
}

fn remove_scratch(outcome: std::io::Result<()>, path: &Path) -> Result<()> {
    match outcome {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Other(format!(
            "removing stale snippet session scratch {}: {error}",
            path.display()
        ))),
    }
}

/// Removes stray top-level files (never directories, never recursing) left in a live session
/// scratch `directory` by a previous run's per-snippet validate calls. See
/// [`purge_stale_session_scratch`] for why this must run before `before` hooks. A directory that
/// does not exist yet (the common case: first run for this fingerprint) is not an error.
fn purge_stale_workspace_scratch_files(directory: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::Other(format!(
                "reading snippet workspace directory {}: {error}",
                directory.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::Other(format!(
                "reading an entry in snippet workspace directory {}: {error}",
                directory.display()
            ))
        })?;
        let is_stale_file = entry.file_type().is_ok_and(|file_type| file_type.is_file());
        if !is_stale_file {
            continue;
        }
        if let Err(error) = std::fs::remove_file(entry.path())
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Error::Other(format!(
                "removing stale snippet scratch file {}: {error}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Sweeps the scratch root [`crate::snippets::scratch::scratch_root`] chose for this spec, removing
/// entries abandoned by a run that was killed before its guards could drop.
///
/// This exists because moving scratch under `.alef/snippets/tmp` pointed
/// [`cleanup_legacy_scratch_directories`] — which only ever reads the top level of
/// `working_directory` — at a set that no longer contains any scratch at all. Without this the
/// only remaining cleanup was in-process `Drop`, which a `SIGINT` skips entirely, so leftovers
/// accumulated in the cache root indefinitely.
///
/// Logged and tolerated rather than propagated, for the same reason as
/// [`purge_stale_session_scratch`]: losing a sweep costs cleanliness, while failing preparation
/// over it would black out exactly the language the sweep exists to keep clean. ~keep
pub(super) fn purge_abandoned_scratch(spec: &SessionSpec, timeout_secs: u64) {
    let root = crate::snippets::scratch::scratch_root(spec.language, &spec.working_directory, spec.manifest.as_deref());
    if let Err(error) = crate::snippets::scratch::purge_stale_scratch_root(&root, timeout_secs) {
        tracing::warn!(
            scratch_root = %root.display(),
            language = %spec.language,
            error = %error,
            "could not purge abandoned snippet scratch"
        );
    }
}

/// Sweeps `.alef-snippet-*` directories left *directly* in `working_directory` by alef versions
/// that predate the single scratch destination. Nothing writes there any more, so this covers only
/// pre-fix leftovers; abandoned scratch from the current layout is [`purge_abandoned_scratch`]'s
/// job. Deliberately keyed on the `.alef-snippet-` prefix and on directories only: this root is
/// the consumer's own source directory, not alef's, so anything less specific would be a delete
/// gate pointed at tracked files. ~keep
pub(super) fn cleanup_legacy_scratch_directories(working_directory: &Path, timeout_secs: u64) -> Result<()> {
    let stale_after = std::time::Duration::from_secs(timeout_secs.saturating_add(60));
    let entries = std::fs::read_dir(working_directory).map_err(|error| {
        Error::Other(format!(
            "reading snippet working directory {}: {error}",
            working_directory.display()
        ))
    })?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::Other(format!(
                    "reading an entry in snippet working directory {}: {error}",
                    working_directory.display()
                )));
            }
        };
        let entry_type = match entry.file_type() {
            Ok(entry_type) => entry_type,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::Other(format!(
                    "reading snippet scratch entry type {}: {error}",
                    entry.path().display()
                )));
            }
        };
        if !entry_type.is_dir() || !entry.file_name().to_string_lossy().starts_with(".alef-snippet-") {
            continue;
        }
        let modified = match entry.metadata() {
            Ok(metadata) => metadata.modified().map_err(|error| {
                Error::Other(format!(
                    "reading snippet scratch modification time {}: {error}",
                    entry.path().display()
                ))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::Other(format!(
                    "reading snippet scratch metadata {}: {error}",
                    entry.path().display()
                )));
            }
        };
        if modified.elapsed().is_ok_and(|age| age >= stale_after)
            && let Err(error) = std::fs::remove_dir_all(entry.path())
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Error::Other(format!(
                "removing stale snippet scratch directory {}: {error}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

