//! Relock every `Cargo.lock` that `alef validate versions` will check, immediately after
//! `sync_versions` rewrites the manifests those locks pin.
//!
//! ~keep alef #148: `sync_versions` bumped every `Cargo.toml` it owned but never refreshed the
//! sibling `Cargo.lock` files, so `alef validate versions` — which discovers lockfiles through
//! a separate, broader enumeration — found the stale pin and failed the release gate. Three
//! consumer releases, in three separate repos, were tagged and pushed with a stale lockfile,
//! failed validation, and never reached crates.io. Discovery
//! here is not re-derived: it calls the exact same
//! `crate::cli::commands::version_manifests::discover_cargo_locks` the validator uses, so the
//! write set and the validate set cannot drift into checking a different set of files again.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::cli::commands::version_manifests::discover_cargo_locks;
use crate::cli::git::tracked_paths_under;

/// Run `cargo update --offline -w` in the directory of every discovered lockfile that is not
/// waiting on a pending release.
///
/// Locks `discover_cargo_locks` marks `blocked_on_publish` are skipped on purpose: they pin a
/// registry dependency at the version being released, so cargo cannot resolve that requirement
/// until the release is live on the registry — an offline update there would just fail (or do
/// nothing). `alef validate versions` already tolerates those rows (`checks_pass`), and this
/// relock step honors the same exemption rather than treating it as something to fix now.
pub(super) fn relock_cargo_lockfiles(canonical: &str) {
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tracked = tracked_paths_under(&workspace_root);
    if tracked.is_none() {
        warn!(
            "version-sync: cannot determine which files are git-tracked (not a git work tree, or `git` is \
             unavailable) — lockfile relock falls back to an unfiltered disk walk and may touch build-staging \
             copies"
        );
    }
    for lock in discover_cargo_locks(&workspace_root, canonical, tracked.as_ref()) {
        if lock.blocked_on_publish.is_some() {
            debug!(lock = %lock.path.display(), "version-sync: skipping relock — blocked on publish");
            continue;
        }
        let Some(dir) = lock.path.parent() else {
            continue;
        };
        info!("Relocking {} after version sync", lock.path.display());
        relock_one(dir, &lock.path);
    }
}

/// Relock the `Cargo.lock` sitting beside a nested, alef-generated `Cargo.toml` (a Ruby, R, or
/// Elixir native-extension manifest -- never a root workspace member) immediately after this
/// write actually changed that manifest's content on disk.
///
/// [`relock_cargo_lockfiles`] above only ever runs from `sync_versions`, the version-bump
/// pipeline. But a nested binding-crate manifest is `generated_header: true` and gets rewritten
/// on every ordinary `alef build`/`alef generate`/`alef scaffold` too -- completely independent
/// of a version bump, whenever a dependency constraint in it changes (a template dependency
/// version bump, an added feature, a config edit). Nothing relocked the sibling lockfile on
/// that path, so `cargo check --locked` against the freshly regenerated manifest could fail
/// immediately with no version bump involved at all -- the manifest widened or tightened a
/// requirement the lockfile's existing pin no longer satisfies. Scoped to `changed_paths`
/// (never a full-tree walk like `relock_cargo_lockfiles`) so a routine build only ever pays for
/// the manifests it actually rewrote, not every lockfile in the repo. ~keep
pub(super) fn relock_lockfiles_beside_changed_manifests(changed_paths: &HashSet<PathBuf>) {
    for path in changed_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            continue;
        }
        let Some(dir) = path.parent() else {
            continue;
        };
        let lock_path = dir.join("Cargo.lock");
        if !lock_path.exists() {
            continue;
        }
        info!("Relocking {} after its generated manifest changed", lock_path.display());
        relock_one(dir, &lock_path);
    }
}

/// Best-effort, like the other lockfile-refresh commands `sync_versions` already runs
/// (`pnpm install`, `composer update`, `mix deps.get`): a missing `cargo` binary or a lockfile
/// that fails to resolve offline must not abort the whole version sync.
fn relock_one(dir: &Path, lock_path: &Path) {
    match std::process::Command::new("cargo")
        .args(["update", "--offline", "-w"])
        .current_dir(dir)
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            warn!(
                lock = %lock_path.display(),
                code = ?status.code(),
                "cargo update --offline -w failed for this lockfile; it may still be stale against \
                 its manifest. Re-run `cargo update` in that directory with network access if a \
                 later `cargo check --locked` rejects it"
            );
        }
        Err(error) => {
            warn!(
                lock = %lock_path.display(),
                %error,
                "could not run cargo update for this lockfile; it may still be stale against its manifest"
            );
        }
    }
}
