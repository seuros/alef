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

use std::path::Path;
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
                "version-sync: cargo update --offline -w failed for this lockfile"
            );
        }
        Err(error) => {
            warn!(
                lock = %lock_path.display(),
                %error,
                "version-sync: could not run cargo update for this lockfile"
            );
        }
    }
}
