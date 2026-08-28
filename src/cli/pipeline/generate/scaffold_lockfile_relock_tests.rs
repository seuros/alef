//! Regression coverage for the nested-lockfile publish blocker: alef regenerates a nested
//! binding-crate `Cargo.toml` (`generated_header: true`, e.g. a Ruby/R/Elixir native-extension
//! manifest) on every ordinary `alef build`/`alef generate`/`alef scaffold`, but nothing used to
//! refresh the `Cargo.lock` sitting beside it. A consumer's `cargo check --locked` against that
//! manifest then failed before building at all, because a dependency constraint in the
//! regenerated manifest no longer matched the lockfile's existing pin.
//!
//! `write_scaffold_files_report` (defined in `super`, this module's parent) now relocks any
//! `Cargo.lock` beside a `Cargo.toml` it actually changed -- see
//! `super::super::version_lockfiles::relock_lockfiles_beside_changed_manifests`. The tests below
//! exercise the wrapper directly with a real, dependency-free crate so `cargo update --offline
//! -w` needs no network and no registry cache, mirroring the existing coverage for the
//! version-bump-only relock path in `version_tests/lockfile_relock.rs`.

use super::*;
use crate::core::backend::GeneratedFile;
use std::path::PathBuf;

const OLD_VERSION: &str = "0.1.0";
const NEW_VERSION: &str = "0.2.0";

fn manifest_file(path: &str, version: &str) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(path),
        content: format!(
            "[workspace]\nmembers = []\n\n[package]\nname = \"sample-native\"\nversion = \"{version}\"\nedition = \
             \"2024\"\n"
        ),
        generated_header: true,
    }
}

fn lib_rs_file(path: &str) -> GeneratedFile {
    GeneratedFile {
        path: PathBuf::from(path),
        content: String::new(),
        generated_header: false,
    }
}

/// The regression itself: a nested manifest's content changing across two ordinary scaffold
/// writes -- no version bump, no `sync_versions` involved -- must leave the sibling `Cargo.lock`
/// relocked to match, not stale beside a manifest alef just rewrote.
#[test]
fn write_scaffold_files_report_relocks_a_changed_nested_manifests_lockfile() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path();
    let manifest_path = "packages/example/native/Cargo.toml";
    let lib_path = "packages/example/native/src/lib.rs";

    write_scaffold_files_report(
        &[manifest_file(manifest_path, OLD_VERSION), lib_rs_file(lib_path)],
        base,
        false,
    )
    .expect("initial scaffold write ok");

    // Written directly to disk, never through alef -- alef never authors lockfiles, only the
    // manifest beside them. This is the stale lock a real `cargo build`/CI run left behind.
    let lock_path = base.join("packages/example/native/Cargo.lock");
    std::fs::write(
        &lock_path,
        format!("version = 4\n\n[[package]]\nname = \"sample-native\"\nversion = \"{OLD_VERSION}\"\n"),
    )
    .expect("seed stale Cargo.lock");

    let new_files = [manifest_file(manifest_path, NEW_VERSION), lib_rs_file(lib_path)];
    let report = write_scaffold_files_report(&new_files, base, false).expect("regenerated scaffold write ok");
    assert!(
        report.changed_paths.contains(&base.join(manifest_path)),
        "the manifest write must be reported as changed: {:?}",
        report.changed_paths
    );

    let manifest = std::fs::read_to_string(base.join(manifest_path)).expect("read regenerated Cargo.toml");
    assert!(
        manifest.contains(&format!("version = \"{NEW_VERSION}\"")),
        "the regenerated manifest must carry the new version, got:\n{manifest}"
    );

    let lock = std::fs::read_to_string(&lock_path).expect("read relocked Cargo.lock");
    assert!(
        lock.contains(&format!("version = \"{NEW_VERSION}\"")),
        "write_scaffold_files_report must relock the sibling Cargo.lock to {NEW_VERSION}, not just \
         rewrite the manifest next to a stale lock, got:\n{lock}"
    );
    assert!(
        !lock.contains(&format!("version = \"{OLD_VERSION}\"")),
        "the stale {OLD_VERSION} pin must be gone from the relocked Cargo.lock, got:\n{lock}"
    );
}

/// Selectivity control: a `Cargo.lock` beside a directory whose `Cargo.toml` did NOT change this
/// run must be left byte-for-byte untouched. Without this, an assertion that only checks the
/// positive case could pass vacuously against a relock that walks every lockfile in the tree
/// rather than only the ones this run actually rewrote.
#[test]
fn write_scaffold_files_report_does_not_touch_a_lockfile_whose_manifest_did_not_change() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path();
    let manifest_path = "packages/example/native/Cargo.toml";
    let lib_path = "packages/example/native/src/lib.rs";
    let untouched_lock_path = base.join("packages/other/native/Cargo.lock");

    write_scaffold_files_report(
        &[manifest_file(manifest_path, OLD_VERSION), lib_rs_file(lib_path)],
        base,
        false,
    )
    .expect("initial scaffold write ok");

    // No `Cargo.toml` at "packages/other/native/" is ever part of `files` in either write below,
    // so this lockfile has nothing to relock against and must stay exactly as seeded.
    std::fs::create_dir_all(untouched_lock_path.parent().expect("has parent")).expect("mkdir");
    const SENTINEL: &str = "version = 4\n\n[[package]]\nname = \"unrelated\"\nversion = \"9.9.9\"\n";
    std::fs::write(&untouched_lock_path, SENTINEL).expect("seed unrelated Cargo.lock");

    write_scaffold_files_report(
        &[manifest_file(manifest_path, NEW_VERSION), lib_rs_file(lib_path)],
        base,
        false,
    )
    .expect("regenerated scaffold write ok");

    let untouched = std::fs::read_to_string(&untouched_lock_path).expect("read unrelated Cargo.lock");
    assert_eq!(
        untouched, SENTINEL,
        "a Cargo.lock beside a manifest this run never wrote must be left byte-for-byte untouched"
    );
}
