//! Build script for `alef`.
//!
//! Besides the Windows stack-size link arg, this captures build provenance — commit sha, working
//! tree state, build time — into `rustc-env` vars that `src/bin_cli/build_info.rs` renders into
//! `alef --version`. Three binaries built from the same `Cargo.toml` version on the same day are
//! otherwise indistinguishable from their own output, which makes any measurement taken with one
//! of them unattributable to a revision. ~keep

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Emitted verbatim wherever git cannot answer. Never an empty string: an empty build id renders
/// as a clean, well-provenanced build, which is precisely the lie this stamp exists to prevent. ~keep
const UNKNOWN: &str = "unknown";

/// Length of the short sha. Fixed here rather than delegating to `git rev-parse --short`, whose
/// output length varies with repository size and would make stamps incomparable across clones. ~keep
const SHORT_SHA_LEN: usize = 12;

const TREE_CLEAN: &str = "clean";
const TREE_DIRTY: &str = "dirty";

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg-bin=alef=/STACK:8388608");
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));
    emit_rerun_triggers(&manifest_dir);
    emit_provenance(&manifest_dir);
}

/// Run `git` in the manifest directory, returning trimmed stdout on success.
///
/// Every failure mode — git absent from PATH, no repository, a crates.io tarball with no `.git`,
/// a non-zero exit, non-UTF-8 output — collapses to `None` so the build never fails over
/// provenance. The caller substitutes [`UNKNOWN`]. ~keep
fn git(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).current_dir(manifest_dir).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Tell cargo which paths invalidate the stamp.
///
/// Emitting any `rerun-if-changed` line replaces cargo's default rule ("rerun when any file in the
/// package changed"), so this set must name every input the stamp depends on:
///
/// * `<gitdir>/HEAD` and the loose ref it points at — the commit sha changes only when one of
///   these moves (commit, checkout, branch switch, reset).
/// * `<common>/packed-refs` — after `git gc` the branch tip lives here and the loose ref file is
///   gone, so watching only the loose path would miss the move.
/// * `<gitdir>/index` — the nearest thing git keeps to a "working tree state" file.
/// * `src`, `build.rs`, `Cargo.toml`, `Cargo.lock` — the compile inputs an uncommitted edit
///   lands in most of the time.
///
/// Only existing paths are emitted: cargo treats a missing `rerun-if-changed` target as changed on
/// every build, which would re-run this script every time and, because the build timestamp differs
/// on each run, force a full recompile of the crate on every build.
///
/// This set does not make the dirty flag exact, and no set can. Cargo decides whether to run this
/// script from file mtimes alone, before anything is in a position to ask git what the tree looks
/// like. Two gaps remain, both known and deliberately unfixed:
///
/// 1. `<gitdir>/index` only moves when git itself writes it. Editing a tracked file and rebuilding
///    without running any git command in between does not touch the index, so a build whose only
///    trigger would have been the index does not re-run.
/// 2. Uncommitted edits confined to unwatched tracked paths (`tests/`, `benches/`, `assets/`,
///    `schemas/`, `hooks/`, `alef.toml`) dirty the tree without touching a watched path, leaving a
///    previously-recorded stamp in place. Those directories are excluded on purpose: watching them
///    would re-run this script — and therefore recompile the whole crate, since the timestamp
///    changes every run — on every edit to a test.
///
/// The asymmetry that follows is the one worth remembering: a `dirty` stamp and a commit sha are
/// trustworthy, because nothing clears them spuriously, while `clean` means only "clean as of the
/// last time a watched path moved" and can be stale. ~keep
fn emit_rerun_triggers(manifest_dir: &Path) {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    for input in ["build.rs", "Cargo.toml", "Cargo.lock", "src"] {
        let path = manifest_dir.join(input);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let Some(git_dir) = git(manifest_dir, &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from) else {
        return;
    };
    let common_dir = git(
        manifest_dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map_or_else(|| git_dir.clone(), PathBuf::from);

    let head = git_dir.join("HEAD");
    let mut watched = vec![head.clone(), git_dir.join("index"), common_dir.join("packed-refs")];
    if let Some(reference) = std::fs::read_to_string(&head)
        .ok()
        .and_then(|contents| contents.trim().strip_prefix("ref: ").map(str::to_string))
    {
        watched.push(common_dir.join(reference));
    }

    for path in watched {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn emit_provenance(manifest_dir: &Path) {
    let commit = git(manifest_dir, &["rev-parse", "HEAD"]);
    let short = commit
        .as_deref()
        .and_then(|sha| sha.get(..SHORT_SHA_LEN))
        .map_or_else(|| UNKNOWN.to_string(), str::to_string);

    println!(
        "cargo:rustc-env=ALEF_BUILD_COMMIT={}",
        commit.as_deref().unwrap_or(UNKNOWN)
    );
    println!("cargo:rustc-env=ALEF_BUILD_COMMIT_SHORT={short}");
    println!("cargo:rustc-env=ALEF_BUILD_TREE_STATE={}", tree_state(manifest_dir));
    println!("cargo:rustc-env=ALEF_BUILD_TIMESTAMP={}", build_timestamp());
}

/// Classify the working tree as `clean`, `dirty`, or [`UNKNOWN`].
///
/// `--no-optional-locks` keeps `git status` from refreshing and rewriting `.git/index`. Without it
/// this script would bump the index mtime on every run, cargo would see a watched path change, and
/// the next build would re-run the script and recompile the crate — forever.
///
/// Untracked files count as dirty. They are a deviation from the committed tree, a new untracked
/// module compiles into the binary like any other, and the tie-break belongs on the safe side:
/// a spurious `dirty` costs an unnecessary `git status`, whereas a spurious `clean` is exactly the
/// failure this stamp exists to prevent. ~keep
fn tree_state(manifest_dir: &Path) -> &'static str {
    match git(manifest_dir, &["--no-optional-locks", "status", "--porcelain"]) {
        // `git()` maps empty output to `None`, so a clean tree is indistinguishable here from git
        // being unavailable. Re-establish the difference by asking a question a repository always
        // answers and an absent/unusable git never does. ~keep
        None if git(manifest_dir, &["rev-parse", "--is-inside-work-tree"]).is_none() => UNKNOWN,
        None => TREE_CLEAN,
        Some(_) => TREE_DIRTY,
    }
}

/// Seconds since the Unix epoch, honoring `SOURCE_DATE_EPOCH` so reproducible-build environments
/// can pin it. Rendered into a human-readable UTC timestamp at the point of display. ~keep
fn build_timestamp() -> String {
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH")
        && epoch.trim().parse::<i64>().is_ok()
    {
        return epoch.trim().to_string();
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or_else(|_| UNKNOWN.to_string(), |elapsed| elapsed.as_secs().to_string())
}
