//! The write boundary against a symlinked ancestor that already exists on disk.
//!
//! `super::contained_output_path`'s lexical pass is checked by `output_containment_tests` in
//! `write.rs`: it runs on the emitted string, before anything exists, and rejects `..`,
//! absoluteness and drive prefixes. These tests cover the case that pass structurally cannot
//! see -- a path where every component is lexically innocent and the *filesystem* is what
//! carries it out of the project, because an ancestor directory beneath `base_dir` is a symlink
//! a repository can ship in its own tracked tree.
//!
//! Unix-only: `std::os::unix::fs::symlink` has no portable equivalent, and creating a directory
//! symlink on Windows needs a separate call and, usually, a privilege the test runner does not
//! have. The production check itself is not gated -- only the ability to *stage* the escape is.
//!
//! The control tests are load-bearing in the other direction: a containment check that rejects
//! every pre-existing ancestor, or every base reached through a symlink, would pass the security
//! test above and break every real build on macOS, where `/tmp` and `/var/folders` (the home of
//! `tempfile::tempdir`) are themselves symlinks. ~keep
#![cfg(unix)]

use super::write_files_report;
use crate::core::backend::GeneratedFile;
use crate::core::config::Language;
use std::path::Path;

const EMITTED: &str = "packages/node/index.ts";
const CONTENT: &str = "export const generated = true;\n";

fn emitted_files() -> Vec<(Language, Vec<GeneratedFile>)> {
    vec![(
        Language::Node,
        vec![GeneratedFile {
            path: EMITTED.into(),
            content: CONTENT.into(),
            generated_header: true,
        }],
    )]
}

fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("symlink");
}

#[test]
fn symlinked_ancestor_directory_is_refused_before_the_write_escapes_the_project() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(base.join("packages")).expect("packages directory");
    std::fs::create_dir(&outside).expect("outside directory");
    symlink(&outside, &base.join("packages/node"));

    let error = write_files_report(&emitted_files(), &base).expect_err("symlinked ancestor must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert!(
        !outside.join("index.ts").exists(),
        "the emitted file reached {} through a symlinked ancestor",
        outside.display()
    );
    // Emptiness, not just the absence of the named file: the refusal has to land ahead of
    // `create_dir_all` and `NamedTempFile::new_in`, and a check that fired after either of them
    // would leave a directory or an abandoned temporary behind out here. ~keep
    let residue: Vec<_> = std::fs::read_dir(&outside)
        .expect("read outside directory")
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    assert!(residue.is_empty(), "write left {residue:?} outside the project");
}

#[test]
fn symlinked_leaf_pointing_outside_the_project_is_refused() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let outside = temporary.path().join("outside");
    std::fs::create_dir_all(base.join("packages/node")).expect("node directory");
    std::fs::create_dir(&outside).expect("outside directory");
    std::fs::write(outside.join("index.ts"), "// foreign\n").expect("outside file");
    symlink(&outside.join("index.ts"), &base.join(EMITTED));

    let error = write_files_report(&emitted_files(), &base).expect_err("symlinked leaf must be refused");

    assert!(error.to_string().contains("not contained"), "{error}");
    assert_eq!(
        std::fs::read_to_string(outside.join("index.ts")).expect("outside file"),
        "// foreign\n"
    );
}

#[test]
fn ordinary_existing_output_directory_still_accepts_writes() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    std::fs::create_dir_all(base.join("packages/node")).expect("node directory");

    write_files_report(&emitted_files(), &base).expect("ordinary existing directory must still be written to");

    let written = std::fs::read_to_string(base.join(EMITTED)).expect("generated file");
    assert!(written.contains("export const generated = true;"), "{written}");
}

#[test]
fn symlinked_ancestor_that_stays_inside_the_project_is_accepted() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let base = temporary.path().join("base");
    let real = base.join("real-node");
    std::fs::create_dir_all(&real).expect("real node directory");
    std::fs::create_dir_all(base.join("packages")).expect("packages directory");
    symlink(&real, &base.join("packages/node"));

    write_files_report(&emitted_files(), &base).expect("a symlink that stays inside the project is not an escape");

    let written = std::fs::read_to_string(real.join("index.ts")).expect("generated file");
    assert!(written.contains("export const generated = true;"), "{written}");
}

#[test]
fn base_directory_reached_through_a_symlink_is_not_an_escape() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let real_root = temporary.path().join("real-root");
    std::fs::create_dir_all(real_root.join("base/packages/node")).expect("node directory");
    let linked_root = temporary.path().join("linked-root");
    symlink(&real_root, &linked_root);

    // The whole `base_dir` is addressed through a symlink, exactly as it is for every consumer
    // whose checkout sits under a symlinked home, volume or `/tmp`. Containment must be judged
    // between canonical paths on both sides, or this legitimate write is rejected. ~keep
    let base = linked_root.join("base");
    write_files_report(&emitted_files(), &base).expect("a symlinked base_dir must not be treated as an escape");

    let written = std::fs::read_to_string(real_root.join("base").join(EMITTED)).expect("generated file");
    assert!(written.contains("export const generated = true;"), "{written}");
}
