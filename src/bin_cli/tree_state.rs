//! Classifies the working tree a binary was built from as `clean`, `dirty`, or `unknown`.
//!
//! This module has two consumers and exists as a file for exactly that reason. `build.rs` pulls it
//! in with `#[path = "src/bin_cli/tree_state.rs"] mod tree_state;` to stamp
//! `ALEF_BUILD_TREE_STATE` at compile time, and the crate compiles it normally so `cargo test
//! --lib` can drive [`classify`] against real git repositories. A build script cannot depend on
//! the crate it builds, so sharing the source file is the only way the shipped classifier and the
//! tested classifier are the same code rather than two implementations that agree until they
//! don't. Keep it `std`-only: `build.rs` has no access to this crate's dependencies. ~keep

use std::path::Path;
use std::process::Command;

/// Working tree matches `HEAD` in every tracked path.
pub const TREE_CLEAN: &str = "clean";

/// At least one tracked path differs from `HEAD`, so the build is not reproducible from any commit.
pub const TREE_DIRTY: &str = "dirty";

/// Git could not answer — not installed, no repository (a crates.io tarball), or no commit yet.
pub const TREE_UNKNOWN: &str = "unknown";

/// `git diff --quiet` exit code meaning "no differences".
const GIT_DIFF_NO_DIFFERENCES: i32 = 0;

/// `git diff --quiet` exit code meaning "differences found". Every other code is a failure to
/// answer (128 outside a repository or with an unborn `HEAD`, 129 for a usage error), which is
/// [`TREE_UNKNOWN`] rather than either verdict. ~keep
const GIT_DIFF_DIFFERENCES: i32 = 1;

/// Classify the working tree at `manifest_dir`.
///
/// The comparison is `git diff HEAD`: **tracked** paths only, index and working tree both, so a
/// staged addition or deletion counts. Untracked files deliberately do not.
///
/// Untracked files used to count, on the theory that an untracked module compiles into the binary
/// like any other. It cannot: reaching the compiler requires a `mod`/`include!` chain rooted at
/// `src/lib.rs`, and every link in that chain is tracked, so any untracked source that actually
/// affects the build drags a tracked modification along with it and is caught here anyway. What
/// counting untracked files did catch was `.cargo-ok` — the completion marker Cargo drops into
/// every `~/.cargo/git/checkouts/…` tree — which made *every* `cargo install --git` build stamp
/// itself `dirty`. A warning that fires on every install is a warning nobody reads, and this one
/// has to stay readable: it is the only thing standing between a dirty binary and committed output
/// attributed to a commit that cannot reproduce it.
///
/// Denylisting `.cargo-ok` would fix that single filename and wait for the next tool to drop the
/// next marker. Asking about tracked content instead is not a list and never needs extending.
///
/// `--no-optional-locks` keeps git from refreshing and rewriting `.git/index`. Without it this
/// bumps the index mtime on every run, cargo sees a watched path change, and the next build
/// re-runs the script and recompiles the crate — forever. ~keep
pub fn classify(manifest_dir: &Path) -> &'static str {
    match tracked_diff_exit_code(manifest_dir) {
        Some(GIT_DIFF_NO_DIFFERENCES) => TREE_CLEAN,
        Some(GIT_DIFF_DIFFERENCES) => TREE_DIRTY,
        _ => TREE_UNKNOWN,
    }
}

/// Exit code of `git diff --quiet HEAD`, or `None` when git could not be spawned at all.
///
/// Output is captured rather than inherited: outside a repository git prints its full usage block
/// to stderr, and a build script has no business dumping that into a build log. ~keep
fn tracked_diff_exit_code(manifest_dir: &Path) -> Option<i32> {
    Command::new("git")
        .args(["--no-optional-locks", "diff", "--quiet", "HEAD"])
        .current_dir(manifest_dir)
        .output()
        .ok()?
        .status
        .code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// The Cargo checkout-completion marker that caused every `cargo install --git` build to stamp
    /// itself dirty. Named here so the regression is legible, never matched against. ~keep
    const CARGO_OK: &str = ".cargo-ok";

    const TRACKED_FILE: &str = "tracked.txt";

    /// Run git hermetically: no user or system config, so a developer's `commit.gpgsign`,
    /// `core.autocrlf`, or hooks cannot decide whether this test passes. ~keep
    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "alef test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "alef test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .expect("git is available");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A repository with one committed file and nothing else.
    fn repo_with_one_commit() -> (TempDir, PathBuf) {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().to_path_buf();
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        fs::write(root.join(TRACKED_FILE), "committed contents\n").expect("write tracked file");
        git(&root, &["add", TRACKED_FILE]);
        git(&root, &["commit", "--quiet", "--no-gpg-sign", "-m", "initial"]);
        (temp, root)
    }

    /// Whether `git status --porcelain` sees anything — the question the old implementation asked.
    /// Used only to prove a fixture really is in the state its test claims. ~keep
    fn porcelain_status_is_nonempty(dir: &Path) -> bool {
        let output = Command::new("git")
            .args(["--no-optional-locks", "status", "--porcelain"])
            .current_dir(dir)
            .output()
            .expect("git is available");
        !output.stdout.is_empty()
    }

    #[test]
    fn pristine_checkout_reports_clean() {
        let (_temp, root) = repo_with_one_commit();
        assert_eq!(classify(&root), TREE_CLEAN);
    }

    /// The regression. A `cargo install --git` checkout differs from its commit by exactly one
    /// untracked file, `.cargo-ok`, and must not be called dirty for it.
    #[test]
    fn untracked_files_alone_report_clean() {
        let (_temp, root) = repo_with_one_commit();
        fs::write(root.join(CARGO_OK), "ok").expect("write cargo marker");
        fs::write(root.join("stray.log"), "noise\n").expect("write stray file");

        assert!(
            porcelain_status_is_nonempty(&root),
            "fixture is vacuous: git does not see the untracked files this test is about"
        );
        assert_eq!(
            classify(&root),
            TREE_CLEAN,
            "untracked files must not make a checkout dirty"
        );
    }

    /// The half that proves the check still checks: relaxing untracked files must not relax
    /// tracked ones. Without this, [`classify`] returning [`TREE_CLEAN`] unconditionally would
    /// pass the suite.
    #[test]
    fn modified_tracked_file_still_reports_dirty() {
        let (_temp, root) = repo_with_one_commit();
        fs::write(root.join(TRACKED_FILE), "edited contents\n").expect("edit tracked file");
        assert_eq!(classify(&root), TREE_DIRTY);
    }

    #[test]
    fn deleted_tracked_file_still_reports_dirty() {
        let (_temp, root) = repo_with_one_commit();
        fs::remove_file(root.join(TRACKED_FILE)).expect("delete tracked file");
        assert_eq!(classify(&root), TREE_DIRTY);
    }

    /// `git add` makes a new file tracked, so it is now content the commit does not contain.
    #[test]
    fn staged_new_file_reports_dirty() {
        let (_temp, root) = repo_with_one_commit();
        fs::write(root.join("added.rs"), "fn added() {}\n").expect("write new file");
        git(&root, &["add", "added.rs"]);
        assert_eq!(classify(&root), TREE_DIRTY);
    }

    /// A tracked edit is not laundered by unrelated untracked noise sitting beside it.
    #[test]
    fn untracked_files_do_not_mask_a_tracked_modification() {
        let (_temp, root) = repo_with_one_commit();
        fs::write(root.join(CARGO_OK), "ok").expect("write cargo marker");
        fs::write(root.join(TRACKED_FILE), "edited contents\n").expect("edit tracked file");
        assert_eq!(classify(&root), TREE_DIRTY);
    }

    /// A crates.io tarball has no `.git`. It must say `unknown` — not `clean`, which would read as
    /// a provenanced build.
    #[test]
    fn directory_outside_a_repository_reports_unknown() {
        let temp = TempDir::new().expect("temp dir");
        assert_eq!(classify(temp.path()), TREE_UNKNOWN);
    }

    /// An initialized repository with no commit has no `HEAD` to compare against, so there is no
    /// revision to call the tree clean or dirty relative to.
    #[test]
    fn repository_without_a_commit_reports_unknown() {
        let temp = TempDir::new().expect("temp dir");
        git(temp.path(), &["init", "--quiet", "--initial-branch=main"]);
        assert_eq!(classify(temp.path()), TREE_UNKNOWN);
    }
}
