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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::cli::commands::version_manifests::discover_cargo_locks;
use crate::cli::git::tracked_paths_under;

use super::collect_alef_headered_paths;
use super::lock_freshness::{StaleLockFinding, stale_lock_findings};

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

/// Retry a relock for every discovered lock still reporting `blocked_on_publish`, regardless of
/// whether this `sync_versions` invocation itself rewrote any manifest.
///
/// ~keep alef #1528: [`relock_cargo_lockfiles`] above only runs when the caller's own
/// `any_cargo_toml_modified` is true, and `blocked_on_publish` is re-derived fresh, on every
/// call, purely from whatever the *current* lock and manifest disagree on -- it has no memory of
/// when that disagreement first appeared. Skipping it at bump time is correct (an offline update
/// cannot resolve a version that has not published yet), but that correctness is also the trap:
/// once the bump run's own manifest write lands, every later `sync_versions` call — including the
/// one `alef generate` fires automatically on every ordinary regen — finds nothing left to change
/// and never calls `relock_cargo_lockfiles` again, so a lock left `blocked_on_publish` the day it
/// was bumped stays reported that way forever, long after the release it was waiting on actually
/// published. This measured against four consumer repos: `test_apps/rust`'s own self-dependency
/// pin was the one stale directory every affected repo shared. This pass is unconditional
/// specifically to close that gap: cheap when nothing is blocked (one discovery walk, zero
/// `cargo` invocations), and a genuine retry — not a second permanent skip — for anything that
/// is. [`relock_one`]'s existing best-effort handling already absorbs a lock that is still,
/// correctly, unresolvable.
pub(super) fn retry_blocked_lockfiles(canonical: &str) {
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tracked = tracked_paths_under(&workspace_root);
    for lock in discover_cargo_locks(&workspace_root, canonical, tracked.as_ref()) {
        let Some(waiting_on) = lock.blocked_on_publish.as_deref() else {
            continue;
        };
        let Some(dir) = lock.path.parent() else {
            continue;
        };
        info!(
            lock = %lock.path.display(),
            waiting_on,
            "version-sync: retrying relock for a lock previously blocked on a pending release"
        );
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelockMode {
    Offline,
    Online,
}

#[derive(Clone, Copy, Debug)]
struct CargoStatus {
    successful: bool,
    code: Option<i32>,
}

impl CargoStatus {
    fn from_exit_status(status: std::process::ExitStatus) -> Self {
        Self {
            successful: status.success(),
            code: status.code(),
        }
    }

    #[cfg(test)]
    fn success() -> Self {
        Self {
            successful: true,
            code: Some(0),
        }
    }

    #[cfg(test)]
    fn failed(code: Option<i32>) -> Self {
        Self {
            successful: false,
            code,
        }
    }
}

#[derive(Debug)]
enum RelockFailure {
    OfflineCommand(std::io::Error),
    OnlineCommand {
        offline_code: Option<i32>,
        error: std::io::Error,
    },
    BothResolvers {
        offline_code: Option<i32>,
        online_code: Option<i32>,
    },
}

fn attempt_relock_with<F>(mut run: F) -> Result<RelockMode, RelockFailure>
where
    F: FnMut(RelockMode) -> std::io::Result<CargoStatus>,
{
    let offline = run(RelockMode::Offline).map_err(RelockFailure::OfflineCommand)?;
    if offline.successful {
        return Ok(RelockMode::Offline);
    }

    let online = run(RelockMode::Online).map_err(|error| RelockFailure::OnlineCommand {
        offline_code: offline.code,
        error,
    })?;
    if online.successful {
        return Ok(RelockMode::Online);
    }

    Err(RelockFailure::BothResolvers {
        offline_code: offline.code,
        online_code: online.code,
    })
}

fn relock_args(mode: RelockMode) -> &'static [&'static str] {
    match mode {
        RelockMode::Offline => &["update", "--offline", "-w"],
        RelockMode::Online => &["update", "-w"],
    }
}

/// Best-effort, like the other lockfile-refresh commands `sync_versions` already runs
/// (`pnpm install`, `composer update`, `mix deps.get`): a missing `cargo` binary or a lockfile
/// that cannot resolve must not abort the whole version sync. Try the local registry cache first,
/// then retry with registry access when that cache cannot satisfy a newly generated constraint.
/// ~keep
fn relock_one(dir: &Path, lock_path: &Path) {
    let outcome = attempt_relock_with(|mode| {
        std::process::Command::new("cargo")
            .args(relock_args(mode))
            .current_dir(dir)
            .status()
            .map(CargoStatus::from_exit_status)
    });

    match outcome {
        Ok(RelockMode::Offline) => {}
        Ok(RelockMode::Online) => {
            info!(
                lock = %lock_path.display(),
                "Relocked with registry access after the offline attempt failed"
            );
        }
        Err(RelockFailure::OfflineCommand(error)) => {
            warn!(
                lock = %lock_path.display(),
                %error,
                "could not run cargo update for this lockfile; it may still be stale against its manifest"
            );
        }
        Err(RelockFailure::OnlineCommand { offline_code, error }) => {
            warn!(
                lock = %lock_path.display(),
                ?offline_code,
                %error,
                "cargo update failed offline, then the registry-enabled retry could not run; the lockfile may \
                 still be stale against its manifest"
            );
        }
        Err(RelockFailure::BothResolvers {
            offline_code,
            online_code,
        }) => {
            warn!(
                lock = %lock_path.display(),
                ?offline_code,
                ?online_code,
                "cargo update -w failed both offline and with registry access; the lockfile may still be stale \
                 against its manifest. Resolve the dependency conflict in that directory before running \
                 `cargo check --locked`"
            );
        }
    }
}

/// Whether `finding` is explained by a lock [`discover_cargo_locks`] already reports
/// `blocked_on_publish`: the disagreement is exactly this crate's own pending, not-yet-published
/// version, the one case `validate_versions::checks_pass` already tolerates. Any OTHER
/// disagreement reachable from the same lock -- a stale third-party pin reached through a
/// hand-written path dependency, [`super::lock_freshness`]'s own founding `tower-http` incident --
/// is not explained by this and must still fail the gate.
fn explained_by_pending_publish(finding: &StaleLockFinding, blocked: &HashMap<PathBuf, String>) -> bool {
    let Some(waiting_on) = blocked.get(&finding.lock) else {
        return false;
    };
    waiting_on.split('@').next() == Some(finding.dependency.as_str())
}

/// Fail `alef validate versions` -- the release gate the `consumer-release-gates` skill has every
/// consumer run before tagging -- on a committed `Cargo.lock` that cannot resolve a requirement
/// reachable from a manifest alef generated, unless that exact disagreement is this release's own
/// pending version still waiting to publish.
///
/// ~keep alef #1528: [`super::check_generated_lock_freshness`] already detects this drift
/// correctly (consumers praised it), but it only ever runs from inside `alef generate`/`alef
/// all`. A drift whose cause lives entirely in a hand-written dependency alef neither generates
/// nor watches (see [`super::lock_freshness`]'s module doc -- the `tower-http` incident this
/// module was built for) never touches a byte alef owns, so nothing prompts anyone to re-run a
/// regen before cutting the next release, and the diagnostic never gets a chance to fire: the
/// affected consumer repos tagged and pushed with the stale lock already committed, and `cargo
/// build --locked` was the first thing to notice, in CI, after the fact. Reusing the exact same
/// read-only `stale_lock_findings` here -- never a second, independently-derived lock-freshness
/// rule -- means the two call sites can only ever agree on what counts as stale; the sole
/// difference is that a pending-release row (which `stale_lock_findings` alone cannot tell apart
/// from a genuinely abandoned one) is cross-checked against `discover_cargo_locks`'s
/// `blocked_on_publish` and tolerated here, exactly as `validate_versions::checks_pass` already
/// tolerates it.
pub(crate) fn check_release_lock_freshness(workspace_root: &Path, canonical: &str) -> Option<anyhow::Error> {
    let tracked = tracked_paths_under(workspace_root);
    let blocked: HashMap<PathBuf, String> = discover_cargo_locks(workspace_root, canonical, tracked.as_ref())
        .into_iter()
        .filter_map(|lock| lock.blocked_on_publish.map(|waiting_on| (lock.path, waiting_on)))
        .collect();

    let mut manifest_dirs: HashSet<PathBuf> = HashSet::new();
    for path in collect_alef_headered_paths(workspace_root) {
        if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Some(dir) = path.parent() {
            manifest_dirs.insert(dir.to_path_buf());
        }
    }

    let mut findings: Vec<StaleLockFinding> = Vec::new();
    for dir in &manifest_dirs {
        findings.extend(
            stale_lock_findings(dir)
                .into_iter()
                .filter(|finding| !explained_by_pending_publish(finding, &blocked)),
        );
    }
    if findings.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(release_lock_message(&findings)))
}

/// Render the operator-facing failure, mirroring [`super::lock_freshness`]'s own message shape
/// (dependency, requirement, lock, remedy) so the two checks read as one family rather than two
/// independently-worded errors for the same underlying defect.
fn release_lock_message(findings: &[StaleLockFinding]) -> String {
    let mut message = format!(
        "{} committed Cargo.lock pin(s) cannot satisfy a requirement reachable from a manifest alef \
         generated, and this is not this release's own pending, not-yet-published version. `cargo \
         metadata --locked` (and every `cargo build --locked` / CI job) will fail in these \
         directories once this release is tagged and pushed. Alef does not author lockfiles, so \
         this is reported rather than rewritten:",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required as `{}` by {}, but the lock pins only {}. Fix with: cargo \
             update --manifest-path {} -p {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.locked_versions.join(", "),
            finding
                .lock
                .parent()
                .unwrap_or(Path::new("."))
                .join("Cargo.toml")
                .display(),
            finding.dependency,
        ));
    }
    message
}

#[cfg(test)]
#[path = "version_lockfiles_tests.rs"]
mod tests;
