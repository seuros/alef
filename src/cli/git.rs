//! Git-tracked-ness probes shared by commands that discover files by walking the disk.
//!
//! A disk walk cannot tell a consumer's committed file from a build tool's staged copy of
//! it: gem packaging stages `packages/<lang>/…` into `tmp/`, Python builds mirror sources
//! into `build/lib/`, and every copy carries the original's bytes verbatim. Tracked-ness is
//! the signal that separates the two, so any discovery whose result feeds a gate or a
//! rewrite belongs behind this module rather than behind a directory-name blocklist. ~keep
//!
//! Two probes, because reading and writing want opposite errors.
//!
//! [`tracked_paths_under`] answers *is this file committed*, and suits a route that reports on
//! or deletes what it finds: the cost of omitting an uncommitted file is a missed row, and the
//! cost of including a staged copy is a phantom failure or a lost file.
//!
//! [`IgnoreFilter`] answers *would git ignore this file*, and suits a route that rewrites what
//! it finds. Requiring *tracked* there would be wrong in the common case: `alef generate`
//! emits files that are legitimately untracked until the consumer commits them, and the
//! version sync that runs right afterwards must still stamp them. Ignored-ness is the property
//! that actually describes the harm — an edit to a gitignored path is invisible to review and
//! disappears on the next clean — and every tracked file is by construction not ignored, so
//! this is the strictly narrower cut rather than a second opinion on the same question. ~keep

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Git-tracked files under `root`, or `None` when tracked-ness cannot be determined (`root` is
/// not inside a git work tree, or the `git` binary is unavailable). Returned paths are absolute,
/// so they compare directly against the output of a `glob`/`walkdir` pass over the same root.
///
/// Callers choose their own policy for `None`: a route that *deletes* must refuse to proceed,
/// while a read-only report may fall back to its unfiltered walk.
pub fn tracked_paths_under(root: &Path) -> Option<HashSet<PathBuf>> {
    // `-- .` scopes the listing to `root` itself: `git ls-files` with no pathspec lists every
    // tracked file in the whole repository (just displayed relative to `-C`'s directory), which
    // would make this needlessly scan a consumer's entire monorepo on every call. ~keep
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--", "."])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut tracked = HashSet::new();
    for entry in output.stdout.split(|&byte| byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let relative = std::str::from_utf8(entry).ok()?;
        tracked.insert(root.join(relative));
    }
    Some(tracked)
}

/// Whether a discovered path may be rewritten, judged by git's ignore rules rather than by the
/// name of the directory it sits in.
///
/// Built once per command and consulted at every glob/walk-discovered write site. When git
/// cannot answer — no work tree, or no `git` binary — every path is permitted, so the command
/// degrades to exactly the behaviour it had before this filter existed rather than writing
/// nothing. ~keep
pub struct IgnoreFilter {
    root: PathBuf,
    /// Absolute ignored files, plus absolute ignored directory roots that stand for everything
    /// beneath them. `None` means git could not answer and the filter permits everything.
    ignored: Option<Vec<PathBuf>>,
}

impl IgnoreFilter {
    /// Probe git's ignore rules for `root`.
    pub fn for_root(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            ignored: ignored_entries_under(root),
        }
    }

    /// Probe git's ignore rules for the process working directory, which is the workspace root
    /// for every command that discovers files with working-directory-relative glob patterns.
    pub fn for_current_dir() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::for_root(&root)
    }

    /// Whether git could answer at all. A caller that wants to say so in its own log can ask;
    /// the filter itself stays silent so a per-path check does not spam a warning per file.
    pub fn is_degraded(&self) -> bool {
        self.ignored.is_none()
    }

    /// Whether `path` may be written. Relative paths are resolved against the filter's root,
    /// which is how glob results discovered from working-directory-relative patterns arrive.
    pub fn allows(&self, path: &Path) -> bool {
        let Some(ignored) = self.ignored.as_ref() else {
            return true;
        };
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        !ignored.iter().any(|entry| absolute.starts_with(entry))
    }

    /// Expand `pattern` and drop every match git ignores.
    ///
    /// Discovery and the ignore check are one call deliberately: the defect this exists to stop
    /// is a write site that globs and forgets to ask, and a site that cannot glob without asking
    /// cannot forget. ~keep
    pub fn glob(&self, pattern: &str) -> Vec<PathBuf> {
        glob::glob(pattern)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|path| {
                if self.allows(path) {
                    return true;
                }
                tracing::debug!(
                    path = %path.display(),
                    pattern,
                    "skipping a git-ignored match: it is build staging or another disposable copy, \
                     and rewriting it would produce an edit nobody reviews that vanishes on clean"
                );
                false
            })
            .collect()
    }
}

/// Absolute paths under `root` that git ignores: ignored loose files, plus ignored directories
/// collapsed to their root so a whole staging tree costs one entry.
///
/// `None` when git cannot answer (`root` is not inside a work tree, or `git` is unavailable).
fn ignored_entries_under(root: &Path) -> Option<Vec<PathBuf>> {
    // `--others --ignored` lists only files git does not track, so a tracked file that also
    // matches an ignore rule never appears here and stays writable. `--directory` collapses a
    // wholly-ignored directory into one entry, which is both cheaper than enumerating a
    // `target/` tree and what makes the prefix test below correct. `-- .` scopes the listing
    // the same way `tracked_paths_under` does. ~keep
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "--",
            ".",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut ignored = Vec::new();
    for entry in output.stdout.split(|&byte| byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let relative = std::str::from_utf8(entry).ok()?;
        // A directory entry arrives with a trailing separator; `Path::starts_with` matches whole
        // components, so trimming it makes the same test serve files and directory roots. ~keep
        ignored.push(root.join(relative.trim_end_matches('/')));
    }
    Some(ignored)
}
