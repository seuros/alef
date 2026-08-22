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

/// Which of `paths` (absolute) git considers ignored, answered with one `git check-ignore`
/// invocation for the whole list rather than one process per path.
///
/// [`gitignored_dirs`] answers a different question: it collapses a WHOLLY ignored subtree
/// to its topmost ignored directory via `ls-files --directory`, so it only ever reports a
/// directory that itself has nothing tracked inside it. A pattern like `e2e/c/test_*` ignores
/// individual files one at a time inside a directory that stays tracked (it still holds
/// `.gitignore`, `Makefile`, `main.c`) -- that directory never becomes wholly ignored, so
/// `gitignored_dirs` reports nothing for it at all (see this module's tests for the proof,
/// not an assumption). Answering "is this exact path ignored" needs `check-ignore` directly.
///
/// `--stdin -z` feeds every candidate through one process on a background writer thread
/// (avoids a deadlock: with thousands of paths, both this process's stdin pipe and git's
/// stdout pipe fill against their OS buffer limit before all input is written, so writing
/// synchronously before reading any output can hang), which is what keeps this practical for
/// a consumer repo with thousands of managed paths instead of thousands of subprocesses.
///
/// Falls back to an empty set -- never an error -- under the same conditions as
/// `gitignored_dirs`: not a git work tree, git missing from `$PATH`, or any other reason git
/// cannot answer (exit code 128, e.g. "not a git repository"). Exit code 1 ("ran fine, none of
/// the given paths are ignored") is not a failure and is handled the same as exit code 0.
/// Callers must read an empty result as "unknown" when git is unanswerable, not "confirmed
/// clean" -- today's only caller only downgrades a report's wording, never anything
/// destructive. ~keep
pub(crate) fn gitignored_paths(base_dir: &Path, paths: &[PathBuf]) -> HashSet<PathBuf> {
    if paths.is_empty() {
        return HashSet::new();
    }
    let relative: Vec<(String, &PathBuf)> = paths
        .iter()
        .map(|path| {
            let display = path
                .strip_prefix(base_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();
            (display, path)
        })
        .collect();

    let Ok(mut child) = std::process::Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return HashSet::new();
    };
    let Some(mut stdin) = child.stdin.take() else {
        return HashSet::new();
    };
    let input: Vec<u8> = relative
        .iter()
        .flat_map(|(rel, _)| rel.as_bytes().iter().copied().chain(std::iter::once(0u8)))
        .collect();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&input);
    });
    let Ok(output) = child.wait_with_output() else {
        let _ = writer.join();
        return HashSet::new();
    };
    let _ = writer.join();
    if !output.status.success() && output.status.code() != Some(1) {
        return HashSet::new();
    }

    let by_relative: std::collections::HashMap<&str, &PathBuf> =
        relative.iter().map(|(rel, abs)| (rel.as_str(), *abs)).collect();
    output
        .stdout
        .split(|&byte| byte == 0)
        .filter(|chunk| !chunk.is_empty())
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter_map(|rel| by_relative.get(rel).map(|abs| (*abs).clone()))
        .collect()
}

/// Split `missing` -- absolute display paths of alef-managed files absent from disk, as
/// [`super::helpers::missing_managed_paths`] produces -- into plainly absent paths (`alef
/// generate` will write them) and absent-AND-gitignored paths, for which generation can never
/// help: the file gets written, the ignore rule discards it again before commit, and the very
/// next `alef verify` reports it missing again, forever. See [`gitignored_paths`] for why
/// `gitignored_dirs` cannot answer this.
pub(crate) fn split_missing_by_gitignore(base_dir: &Path, missing: &[String]) -> (Vec<String>, Vec<String>) {
    let paths: Vec<PathBuf> = missing.iter().map(PathBuf::from).collect();
    let ignored = gitignored_paths(base_dir, &paths);
    let mut absent = Vec::new();
    let mut absent_gitignored = Vec::new();
    for (path, display) in paths.into_iter().zip(missing.iter()) {
        if ignored.contains(&path) {
            absent_gitignored.push(display.clone());
        } else {
            absent.push(display.clone());
        }
    }
    (absent, absent_gitignored)
}

#[cfg(test)]
mod tests;
