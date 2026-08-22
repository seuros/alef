use super::{gitignored_dirs, gitignored_paths, split_missing_by_gitignore};
use std::path::Path;
use std::process::Command;

fn init_repo(base: &Path) {
    let status = Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["init", "--quiet"])
        .status()
        .expect("git init must run");
    assert!(status.success(), "git init must succeed");
}

/// Reproduces the html-to-markdown case this module exists to fix: a zig package manager's
/// local dependency-fetch cache, several directories deep, gitignored by a pattern with no
/// interior slash (matches at any depth, exactly like `test_apps/zig/zig-pkg/` in that repo's
/// own `.gitignore`).
#[test]
fn gitignored_dirs_reports_a_nested_dependency_cache_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    init_repo(base);
    std::fs::write(base.join(".gitignore"), "zig-pkg/\n").expect("write .gitignore");
    std::fs::create_dir_all(base.join("test_apps/zig/zig-pkg/fetched-pkg/src")).expect("mkdir");
    std::fs::write(
        base.join("test_apps/zig/zig-pkg/fetched-pkg/src/main.zig"),
        "// alef:hash:deadbeef\n",
    )
    .expect("write fetched file");

    let ignored = gitignored_dirs(base);
    assert!(
        ignored.contains(Path::new("test_apps/zig/zig-pkg")),
        "expected the gitignored dependency-cache directory to be reported, got: {ignored:?}"
    );
}

#[test]
fn gitignored_dirs_is_empty_outside_a_git_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ignored = gitignored_dirs(dir.path());
    assert!(
        ignored.is_empty(),
        "a directory with no git repository must yield no ignored dirs, got: {ignored:?}"
    );
}

#[test]
fn gitignored_dirs_does_not_report_a_tracked_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    init_repo(base);
    std::fs::create_dir_all(base.join("packages/python")).expect("mkdir");
    std::fs::write(base.join("packages/python/lib.py"), "# tracked\n").expect("write file");

    let ignored = gitignored_dirs(base);
    assert!(
        !ignored.contains(Path::new("packages/python")),
        "a tracked (non-ignored) directory must not be reported as ignored, got: {ignored:?}"
    );
}

/// Reproduces the tree-sitter-language-pack case this function exists to fix: a per-file
/// pattern (`e2e/c/test_*`) ignores individual files inside a directory that is itself still
/// tracked (it also holds `.gitignore`, `Makefile`, `main.c`). `gitignored_dirs`'s
/// `--directory` view cannot see this at all -- proven below in the same test, not assumed --
/// because the directory never becomes wholly ignored. `gitignored_paths` must answer the
/// per-file question directly.
#[test]
fn gitignored_paths_finds_a_file_ignored_by_a_pattern_that_leaves_its_directory_tracked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    init_repo(base);
    std::fs::create_dir_all(base.join("e2e/c")).expect("mkdir");
    std::fs::write(base.join("e2e/c/.gitignore"), "test_*\n").expect("write nested .gitignore");
    std::fs::write(base.join("e2e/c/Makefile"), "all:\n").expect("write tracked file");
    std::fs::write(base.join("e2e/c/main.c"), "int main(void) { return 0; }\n").expect("write tracked file");
    std::fs::write(base.join("e2e/c/test_runner.h"), "// runner\n").expect("write ignored file");

    // The caution this test exists to prove, not assume: the directory-level helper cannot
    // see this failure mode at all.
    assert!(
        gitignored_dirs(base).is_empty(),
        "the directory itself is not wholly ignored (it holds tracked Makefile/.gitignore/main.c), so \
         gitignored_dirs must report nothing here -- if it did, gitignored_paths would be redundant"
    );

    let candidates = vec![
        base.join("e2e/c/test_runner.h"),
        base.join("e2e/c/Makefile"),
        base.join("e2e/c/main.c"),
    ];
    let ignored = gitignored_paths(base, &candidates);

    assert!(
        ignored.contains(&base.join("e2e/c/test_runner.h")),
        "the individually gitignored file must be reported, got: {ignored:?}"
    );
    assert!(
        !ignored.contains(&base.join("e2e/c/Makefile")) && !ignored.contains(&base.join("e2e/c/main.c")),
        "tracked, non-ignored siblings must not be reported, got: {ignored:?}"
    );
}

#[test]
fn gitignored_paths_is_empty_for_an_empty_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(gitignored_paths(dir.path(), &[]).is_empty());
}

#[test]
fn gitignored_paths_is_empty_outside_a_git_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let candidate = dir.path().join("some/file.txt");
    let ignored = gitignored_paths(dir.path(), std::slice::from_ref(&candidate));
    assert!(
        ignored.is_empty(),
        "outside a git work tree the query is unanswerable, and must degrade to \"nothing confirmed \
         ignored\" rather than erroring, got: {ignored:?}"
    );
}

/// `split_missing_by_gitignore` is what `find_missing_and_frozen_generated_files` calls to turn a
/// flat `missing_managed_paths` list into the two report sections `alef verify` prints: plainly
/// absent (remedy: `alef generate`) versus absent AND gitignored (remedy: narrow the ignore rule,
/// `alef generate` can never fix it).
#[test]
fn split_missing_by_gitignore_separates_absent_from_absent_and_gitignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    init_repo(base);
    std::fs::create_dir_all(base.join("e2e/c")).expect("mkdir");
    std::fs::write(base.join("e2e/c/.gitignore"), "test_*\n").expect("write nested .gitignore");
    std::fs::write(base.join("e2e/c/Makefile"), "all:\n").expect("write tracked file");

    // Neither candidate exists on disk -- `missing_managed_paths` already filtered to
    // absent paths before this function ever sees them, so this function must not itself
    // consult `exists()`.
    let missing = vec![
        base.join("e2e/c/test_runner.h").display().to_string(),
        base.join("SomeType.java").display().to_string(),
    ];

    let (absent, absent_gitignored) = split_missing_by_gitignore(base, &missing);

    assert_eq!(absent, vec![base.join("SomeType.java").display().to_string()]);
    assert_eq!(
        absent_gitignored,
        vec![base.join("e2e/c/test_runner.h").display().to_string()]
    );
}

/// When git cannot answer at all (no repository), every path must fall back to plain
/// "absent" -- today's behavior -- rather than silently disappearing from the report or
/// erroring the whole command.
#[test]
fn split_missing_by_gitignore_falls_back_to_plain_absent_outside_a_git_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let missing = vec![base.join("SomeType.java").display().to_string()];

    let (absent, absent_gitignored) = split_missing_by_gitignore(base, &missing);

    assert_eq!(absent, missing);
    assert!(absent_gitignored.is_empty());
}
