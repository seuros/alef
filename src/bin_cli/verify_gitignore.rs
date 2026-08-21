//! Gitignore-aware directory pruning for `alef verify`'s disk walk.
//!
//! [`crate::bin_cli::helpers::collect_alef_hashes`] only skips a small, hand-maintained list of
//! build/cache directory *names* (`target`, `node_modules`, ...). That list is necessarily
//! incomplete: it cannot know about a consumer's own dependency-fetch cache (a zig package
//! manager's local package cache under `test_apps/zig/zig-pkg/`, populated with *fetched*
//! `.zig` sources from whatever alef version generated the upstream package release at fetch
//! time) or a build tool's own output directory that happens to copy a real, previously
//! alef-marked file into a tree the walk otherwise has no reason to open (`wasm-pack build`
//! copies the crate's alef-marked `README.md` into `crates/<crate>-wasm/pkg/<target>/README.md`
//! as part of packaging). Both are gitignored, unmanaged content the walk should never have
//! opened -- but a hand-maintained directory-name list only ever grows one incident at a time.
//! Consulting git directly generalizes it to "whatever this repo itself says is ignored"
//! instead. ~keep

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Every directory under `base_dir` that git considers ignored, expressed as paths relative to
/// `base_dir` -- one entry per *outermost* ignored directory, since `--directory` collapses a
/// wholly-ignored subtree to its topmost ignored ancestor instead of listing every file inside,
/// which is exactly the shape the walk needs to prune a directory by name at the point it would
/// otherwise descend into it rather than opening every file inside one at a time.
///
/// Falls back to an empty set -- never an error -- when `base_dir` is not a git work tree or
/// `git` is not on `$PATH`: the walk still has its hand-maintained skip list as a baseline, and
/// this must never turn "unanswerable" into "walk the whole gitignored tree anyway" reading as a
/// hard failure. Mirrors `crate::cli::cache::git_tracks`'s "unanswerable, not wrong" handling. ~keep
pub(crate) fn gitignored_dirs(base_dir: &Path) -> HashSet<PathBuf> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .args(["ls-files", "--others", "--ignored", "--exclude-standard", "--directory"])
        .output()
    else {
        return HashSet::new();
    };
    if !output.status.success() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_suffix('/'))
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests;
