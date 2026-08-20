use super::find_orphaned_generated_files;
use std::collections::HashSet;
use std::path::PathBuf;

fn marked_java_content() -> String {
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    crate::core::hash::inject_hash_line(&header, &"0".repeat(64))
}

/// Case (a): a file a backend stopped producing IS reported. Simulates the
/// `NodeContext.java`/`HtmlVisitor.java`/`VisitorBridge.java` case this module exists to catch --
/// the file is still on disk, still carries alef's marker, but the current run's managed-path set
/// (empty here, standing in for "no backend emits this anymore") does not include it.
#[test]
fn reports_a_marked_file_the_current_run_no_longer_produces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let dropped = package_dir.join("VisitorBridge.java");
    std::fs::write(&dropped, marked_java_content()).expect("write dropped file");

    let managed_paths: HashSet<PathBuf> = HashSet::new();
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths);

    assert_eq!(
        orphans,
        vec![dropped.display().to_string()],
        "a marked file absent from the managed-path set must be reported as an orphan"
    );
}

/// Case (b): a current, expected generated file is NOT reported -- it is marked on disk AND
/// present in the managed-path set, so it must never show up alongside a genuine orphan.
#[test]
fn does_not_report_a_marked_file_still_in_the_managed_path_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let current = package_dir.join("Bridge.java");
    std::fs::write(&current, marked_java_content()).expect("write current file");

    let managed_paths: HashSet<PathBuf> = HashSet::from([current.clone()]);
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths);

    assert!(
        orphans.is_empty(),
        "a file still in this run's managed-path set must never be reported: {orphans:?}"
    );
}

/// Case (c): a user-owned file with no alef marker, sitting in the same generated directory as
/// alef-managed output, must NEVER be reported -- this is the case that makes the check safe to
/// ship, since ownership is decided purely by the marker `collect_alef_hashes` already gates on.
#[test]
fn does_not_report_an_unmarked_user_owned_file_in_a_generated_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let current = package_dir.join("Bridge.java");
    std::fs::write(&current, marked_java_content()).expect("write current file");
    let hand_written = package_dir.join("UserExtensions.java");
    std::fs::write(&hand_written, "package dev.demo;\npublic class UserExtensions {}\n").expect("write hand file");

    let managed_paths: HashSet<PathBuf> = HashSet::from([current]);
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths);

    assert!(
        orphans.is_empty(),
        "an unmarked hand-written file must never be reported as an orphan, even sitting in a \
         generated directory: {orphans:?}"
    );
}

/// A known create-once seed (`rust-toolchain.toml`) must never be reported, even though it is
/// absent from `managed_paths` on every run after the one that created it -- `scaffold()` only
/// includes it in a run's surface when the path does not already exist on disk, so this is the
/// expected steady state for every consumer that has one, not a rare edge case.
#[test]
fn does_not_report_the_rust_toolchain_create_once_seed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    let seed = dir.path().join("rust-toolchain.toml");
    std::fs::write(&seed, hashed).expect("write seed");

    let managed_paths: HashSet<PathBuf> = HashSet::new();
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths);

    assert!(
        orphans.is_empty(),
        "rust-toolchain.toml is a documented create-once seed and must never be reported: {orphans:?}"
    );
}

/// A file dropped by crate A but still legitimately owned by crate B in a multi-crate workspace
/// must not be reported, as long as the caller unions both crates' managed paths before calling
/// this function -- exactly as the doc comment requires.
#[test]
fn does_not_report_a_file_owned_by_a_different_crate_once_paths_are_unioned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let owned_by_crate_b = package_dir.join("CrateBOnly.java");
    std::fs::write(&owned_by_crate_b, marked_java_content()).expect("write file");

    // Crate A's managed paths alone do not mention this file; the union with crate B's does.
    let crate_a_managed: HashSet<PathBuf> = HashSet::new();
    let crate_b_managed: HashSet<PathBuf> = HashSet::from([owned_by_crate_b.clone()]);
    let unioned: HashSet<PathBuf> = crate_a_managed.union(&crate_b_managed).cloned().collect();

    let orphans = find_orphaned_generated_files(dir.path(), &unioned);

    assert!(
        orphans.is_empty(),
        "a file owned by another crate in the union must not be reported: {orphans:?}"
    );
}
