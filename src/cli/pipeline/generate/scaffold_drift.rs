//! Informational detection of create-once scaffold files whose on-disk content predates a
//! fix to the template that produced them.
//!
//! `generated_header: false` scaffold files (a zig `build.zig`, a kotlin test seed, a
//! params struct) are written once by `alef scaffold` and are user-owned from then on --
//! see [`super::scaffold::write_scaffold_files_report`]'s ownership guard for why they can
//! never be silently rewritten. That is by design: the consumer may have hand-edited the
//! file, and clobbering that edit would be a worse bug than any template defect. But it
//! also means a real template fix (a missing existence guard, a config-derived value, a
//! corrected struct shape) can never reach a consumer who already scaffolded the file --
//! `alef verify` had no way to even name that condition.
//!
//! # What this checks, and why not the obvious thing
//!
//! A create-once file is *expected* to differ from its template -- that is the entire
//! point of create-once. So "does the on-disk file differ from what alef would generate
//! today" is almost always true and reports nothing actionable; a report that fires on
//! every legitimately-edited file trains readers to skim past it, which is precisely the
//! failure mode `alef diff` hit with `gradle-wrapper.jar` (a binary comparison bug that
//! made one entry drift-report as changed on literally every run, forever, until the
//! comparison itself was fixed to compare bytes correctly).
//!
//! The only evidence available on the consumer's disk that discriminates "the template
//! changed since I scaffolded this" from "I edited this file" is the file's own git
//! history: **has the file ever been touched by a commit after the one that introduced
//! it?** If a file's entire commit history is exactly one commit, nothing in this
//! repository has changed the file since it first appeared here -- so if it *still*
//! differs from what today's template produces, the only remaining explanation is that
//! the template itself moved on. If the file has more than one commit, or was possibly
//! edited before that single commit, this module says nothing at all: the ambiguity is
//! resolved in favour of silence, not a guess.
//!
//! # False-positive / false-negative characteristics (stated honestly)
//!
//! * **False positive**: a consumer who hand-edits a freshly-scaffolded file *before*
//!   making its first commit (or who `git commit --amend`s / squash-merges an edit into
//!   that same commit) leaves exactly one commit in history despite the content being a
//!   real, intentional edit. Git offers no way to see "before that first commit" for a
//!   file that has always had exactly one commit, so this case cannot be distinguished
//!   from genuine template drift with this evidence. It is the class of false positive
//!   this detector accepts.
//! * **False negative** (the safer failure mode, and the deliberate default): any file
//!   with two or more commits is never reported, even when the second commit was a
//!   no-op formatting pass and the file was never meaningfully touched. Untracked files
//!   (zero commits -- not yet committed) are likewise never reported: there is no history
//!   to consult, so the honest answer is "unknown", not "drifted".
//! * A file whose on-disk content already matches what today's template produces is never
//!   reported, regardless of history -- there is nothing to fix.
//! * Binary create-once outputs (anything routed through
//!   [`super::binary::is_base64_binary_output`], e.g. `gradle-wrapper.jar`) are excluded
//!   entirely rather than risk repeating the exact permanently-noisy-entry bug this
//!   module exists to avoid for text files.
//!
//! # Why this is informational only
//!
//! This never fails `alef verify` and is never folded into
//! [`super::super::super::bin_cli::core_commands::verify_outcome::ensure_success`] (or any
//! other hard-fail gate): a create-once file differing from its template is the *expected*
//! steady state for a hand-maintained file, not a defect in the consumer's tree. Reporting
//! it as a hard failure would fail every consumer repo the day this check ships, blocking
//! releases that have nothing to do with the templates flagged. The remedy is also a
//! human decision (review the current template, decide whether to hand-port the fix), not
//! something `alef generate` can act on -- unlike stale bindings, there is no rerun that
//! closes this gap.

use super::normalization::{format_rust_content, normalize_content, normalize_whitespace};
use crate::core::backend::GeneratedFile;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Create-once scaffold paths (relative to `base_dir`) whose content differs from what the
/// current template would produce, and whose git history offers no explanation other than
/// "the template changed since this file was scaffolded" -- see the module doc for exactly
/// what that means and what it does not.
///
/// `scaffold_files` is the freshly-regenerated scaffold set for one crate (as
/// [`super::scaffold::scaffold`] returns it) -- the same in-memory data `alef diff` already
/// computes, reused here rather than regenerated a second time.
pub fn find_create_once_template_drift(scaffold_files: &[GeneratedFile], base_dir: &Path) -> Vec<PathBuf> {
    scaffold_files
        .iter()
        .filter(|file| !file.generated_header)
        .filter(|file| !super::binary::is_base64_binary_output(&file.path))
        .filter(|file| differs_from_template(file, base_dir))
        .filter(|file| untouched_since_scaffold(base_dir, &file.path) == Some(true))
        .map(|file| file.path.clone())
        .collect()
}

/// Whether `file`'s on-disk content (under `base_dir`) differs from what the current
/// template would produce, once both sides go through the same normalization `alef diff`
/// uses (rustfmt for `.rs`, then trailing-whitespace/blank-line normalization) so
/// formatter-only noise is never mistaken for drift. A file that is not on disk at all is
/// not "drifted" -- that is a missing-file condition a different check already covers.
fn differs_from_template(file: &GeneratedFile, base_dir: &Path) -> bool {
    let full_path = base_dir.join(&file.path);
    let Ok(existing) = std::fs::read_to_string(&full_path) else {
        return false;
    };
    let is_rust = file.path.extension().is_some_and(|ext| ext == "rs");
    let generated = normalize_content(&file.path, &file.content);
    let on_disk = if is_rust {
        format_rust_content(&full_path, &existing)
    } else {
        existing
    };
    normalize_whitespace(&on_disk) != normalize_whitespace(&generated)
}

/// `Some(true)`: `relative`'s entire git history under `base_dir` is exactly one commit --
/// nothing has touched the file since whatever commit introduced it, so a difference from
/// today's template can only be explained by the template having moved on.
///
/// `Some(false)`: zero commits (untracked -- no history to consult) or two-or-more commits
/// (possibly edited since scaffolding) -- ambiguous either way, so callers must not report.
///
/// `None`: git could not answer at all (no `git` binary, not a repository). Also must not
/// report -- an export tarball or container without git must never manufacture a claim it
/// has no evidence for.
fn untouched_since_scaffold(base_dir: &Path, relative: &Path) -> Option<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(base_dir)
        .args(["log", "--follow", "--format=%H", "--"])
        .arg(relative)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit_count = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    Some(commit_count == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_git_repo(base_dir: &Path) {
        let status = Command::new("git")
            .arg("-C")
            .arg(base_dir)
            .args(["init", "--quiet"])
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
    }

    fn git_commit_all(base_dir: &Path, message: &str) {
        let status = Command::new("git")
            .arg("-C")
            .arg(base_dir)
            .args(["add", "-A"])
            .status()
            .expect("git add");
        assert!(status.success(), "git add failed");
        let status = Command::new("git")
            .arg("-C")
            .arg(base_dir)
            .args([
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "--quiet",
                "-m",
                message,
            ])
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit failed");
    }

    fn seed_file(base_dir: &Path, relative: &str, content: &str) -> PathBuf {
        let full = base_dir.join(relative);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&full, content).expect("write seed");
        PathBuf::from(relative)
    }

    fn create_once_file(path: PathBuf, content: &str) -> GeneratedFile {
        GeneratedFile {
            path,
            content: content.to_string(),
            generated_header: false,
        }
    }

    /// The main "quiet by default" case: a file whose on-disk content already matches
    /// what the current template produces is never reported, no matter what its git
    /// history looks like -- there is nothing to fix. ~keep
    #[test]
    fn a_create_once_file_matching_the_current_template_is_silent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        let files = vec![create_once_file(relative, "const std = @import(\"std\");\n")];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "unchanged template must never be reported"
        );
    }

    /// The behavior this module exists to add: a file scaffolded once, never touched
    /// again (exactly one commit), that now differs from a template that has since been
    /// fixed -- this is exactly the zig/kotlin/kotlin_android situation the drift report
    /// was built for. ~keep
    #[test]
    fn a_create_once_file_untouched_since_scaffold_and_differing_from_template_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        // The template has since gained an existence guard the scaffolded file predates.
        let files = vec![create_once_file(
            relative.clone(),
            "const std = @import(\"std\");\nif (!exists) return;\n",
        )];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            vec![relative],
            "an untouched file that predates a template fix must be reported"
        );
    }

    /// The false-positive boundary this module deliberately declines to cross: once a
    /// consumer has committed a second time, the diff might be their own edit, so this
    /// must stay silent rather than guess. Pinned explicitly as a known limitation, not
    /// an oversight. ~keep
    #[test]
    fn a_create_once_file_edited_after_scaffolding_is_not_reported_as_template_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");
        std::fs::write(
            dir.path().join("build.zig"),
            "const std = @import(\"std\");\n// hand edit\n",
        )
        .expect("hand edit");
        git_commit_all(dir.path(), "consumer edit");

        let files = vec![create_once_file(
            relative,
            "const std = @import(\"std\");\nif (!exists) return;\n",
        )];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "a file with more than one commit must not be reported -- the second commit might be a \
             legitimate consumer edit, and this detector favors silence over a guess"
        );
    }

    /// An uncommitted (never-committed) create-once file has no history to consult at
    /// all, so it must read the same as "unknown", not "drifted". ~keep
    #[test]
    fn an_uncommitted_create_once_file_is_not_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "build.zig", "const std = @import(\"std\");\n");
        init_git_repo(dir.path());
        // Deliberately never committed.

        let files = vec![create_once_file(
            relative,
            "const std = @import(\"std\");\nif (!exists) return;\n",
        )];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "a file with zero commits has no history to prove drift from, so it must stay silent"
        );
    }

    /// `generated_header: true` files are on the marker/hash rail, not this one -- they
    /// are excluded outright regardless of content or history. ~keep
    #[test]
    fn a_marker_rail_file_is_never_considered_for_create_once_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let relative = seed_file(dir.path(), "src/lib.rs", "// stale\n");
        init_git_repo(dir.path());
        git_commit_all(dir.path(), "scaffold");

        let files = vec![GeneratedFile {
            path: relative,
            content: "// fresh\n".to_string(),
            generated_header: true,
        }];
        let drift = find_create_once_template_drift(&files, dir.path());

        assert_eq!(
            drift,
            Vec::<PathBuf>::new(),
            "marker-rail files are never in scope for this check"
        );
    }
}
