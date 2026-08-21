use super::gitignored_dirs;
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
