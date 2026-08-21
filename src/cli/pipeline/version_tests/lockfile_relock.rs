//! Regression coverage for alef #148: `sync_versions` bumped every `Cargo.toml` it owned
//! without ever refreshing the sibling `Cargo.lock`, so `alef validate versions` — which
//! discovers lockfiles through a separately-derived, broader enumeration — found the stale pin
//! and failed the release gate. Three releases were tagged and pushed with a stale lockfile,
//! failed validation, and never reached crates.io.
//!
//! The fix makes the write side and the read side call the exact same discovery
//! (`crate::cli::commands::version_manifests::discover_cargo_locks`). The test below is the
//! actual deliverable: it does not merely check that a lockfile got new bytes, it runs
//! `sync_versions` and then `validate versions` back to back and asserts the gate passes —
//! proving the write set and the validate set agree, not just that each individually "works".

use super::*;
use crate::cli::commands::validate_versions::{checks_pass, run as validate_versions_run};
use crate::core::config::NewAlefConfig;
use crate::test_support::{CwdGuard, git_add, git_init, write_file};

const OLD_VERSION: &str = "1.2.0";
const NEW_VERSION: &str = "1.2.1";

/// A standalone Rust crate at `e2e/rust`, separate from the root Cargo workspace (`members =
/// []`) with its own `Cargo.lock` — exactly the shape of the e2e/test-app harnesses that went
/// stale in the real incident (`e2e/rust`, `test_apps/rust`, a Ruby native-extension crate).
/// No dependencies, so `cargo update --offline -w` needs no network and no registry cache.
fn build_workspace_with_stale_nested_lock(
    root: &std::path::Path,
) -> (crate::core::config::ResolvedCrateConfig, std::path::PathBuf) {
    git_init(root);
    write_file(
        root,
        "Cargo.toml",
        &format!("[workspace.package]\nversion = \"{NEW_VERSION}\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n"),
    );

    // An empty `[workspace]` table makes the e2e crate its own workspace root, matching what
    // `src/e2e/codegen/rust/cargo_toml.rs` actually generates -- otherwise cargo refuses to
    // treat a directory under the parent workspace's root as an independent crate at all.
    write_file(
        root,
        "e2e/rust/Cargo.toml",
        &format!("[workspace]\n\n[package]\nname = \"mock-server\"\nversion = \"{OLD_VERSION}\"\nedition = \"2024\"\n"),
    );
    write_file(root, "e2e/rust/src/lib.rs", "");
    write_file(
        root,
        "e2e/rust/Cargo.lock",
        &format!("version = 4\n\n[[package]]\nname = \"mock-server\"\nversion = \"{OLD_VERSION}\"\n"),
    );

    let alef_toml = format!(
        concat!(
            "[workspace]\n",
            "languages = [\"rust\"]\n\n",
            "[[crates]]\n",
            "name = \"mylib\"\n",
            "sources = []\n",
            "version_from = \"{cargo_toml}\"\n\n",
            "[crates.e2e]\n",
            "languages = []\n\n",
            "[crates.e2e.call]\n",
            "module = \"mylib\"\n",
            "function = \"parse\"\n",
        ),
        cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
    );
    let alef_toml_path = write_file(root, "alef.toml", &alef_toml);

    git_add(
        root,
        &[
            "Cargo.toml",
            "alef.toml",
            "e2e/rust/Cargo.toml",
            "e2e/rust/Cargo.lock",
            "e2e/rust/src/lib.rs",
        ],
    );

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    (resolved.remove(0), alef_toml_path)
}

/// The regression: `sync_versions` must leave the repo in a state `alef validate versions`
/// accepts, not merely a state where the manifest it directly rewrote looks right.
#[test]
fn sync_versions_relocks_a_nested_lockfile_so_validate_versions_then_passes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let (config, config_path) = build_workspace_with_stale_nested_lock(root);

    {
        let _cwd = CwdGuard::enter(root);
        sync_versions(&config, &config_path, None, true, true, None).expect("sync_versions ok");
    }

    let manifest = std::fs::read_to_string(root.join("e2e/rust/Cargo.toml")).expect("read e2e/rust/Cargo.toml");
    assert!(
        manifest.contains(&format!("version = \"{NEW_VERSION}\"")),
        "e2e/rust/Cargo.toml must be bumped to {NEW_VERSION}, got:\n{manifest}"
    );

    let lock = std::fs::read_to_string(root.join("e2e/rust/Cargo.lock")).expect("read e2e/rust/Cargo.lock");
    assert!(
        lock.contains(&format!("version = \"{NEW_VERSION}\"")),
        "sync_versions must relock e2e/rust/Cargo.lock to {NEW_VERSION}, not just bump the manifest \
         next to a stale lock, got:\n{lock}"
    );
    assert!(
        !lock.contains(&format!("version = \"{OLD_VERSION}\"")),
        "the stale {OLD_VERSION} entry must be gone from the relocked Cargo.lock, got:\n{lock}"
    );

    let checks = validate_versions_run(&config, root, false).expect("validate versions must examine the fixture");
    assert!(
        checks_pass(&checks),
        "alef validate versions must pass immediately after sync_versions -- the write set and the \
         validate set must agree on the same lockfiles: {checks:?}"
    );
}

/// A lockfile `discover_cargo_locks` marks `blocked_on_publish` (a registry dependency pinned
/// at the version being released) must be left alone by the relock step: `cargo update
/// --offline` cannot resolve that requirement before the release is published, and attempting
/// it anyway would just fail. This mirrors `test_apps/rust`, the one stale directory every
/// affected consumer repo shared -- it is supposed to stay stale until publish, which is exactly
/// why `validate_versions::checks_pass` tolerates it rather than the relock step avoiding it.
#[test]
fn sync_versions_does_not_touch_a_lockfile_blocked_on_the_pending_release() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);
    write_file(
        root,
        "Cargo.toml",
        &format!(
            "[workspace.package]\nversion = \"{NEW_VERSION}\"\n\n[workspace]\nresolver = \"2\"\nmembers = \
             [\"crates/sample\"]\n"
        ),
    );
    write_file(
        root,
        "crates/sample/Cargo.toml",
        "[package]\nname = \"sample\"\nversion.workspace = true\n",
    );
    write_file(root, "crates/sample/src/lib.rs", "");

    // `test_apps/rust`'s own manifest already declares the canonical version (as if a previous
    // step had stamped it); only its *lockfile* lags -- the shape a consumer actually hits,
    // since the registry dependency below is what makes the lock unresolvable offline, not a
    // missing manifest bump.
    write_file(
        root,
        "test_apps/rust/Cargo.toml",
        &format!(
            "[workspace]\n\n[package]\nname = \"sample-e2e-rust\"\nversion = \"{NEW_VERSION}\"\n\n\
             [dependencies]\nsample_alias = {{ package = \"sample\", version = \"{NEW_VERSION}\" }}\n"
        ),
    );
    write_file(root, "test_apps/rust/src/main.rs", "fn main() {}\n");
    let stale_lock_version = "1.0.9";
    write_file(
        root,
        "test_apps/rust/Cargo.lock",
        &format!(
            "version = 4\n\n[[package]]\nname = \"sample\"\nversion = \"{OLD_VERSION}\"\n\
             source = \"registry+https://example.invalid/index\"\n\n[[package]]\n\
             name = \"sample-e2e-rust\"\nversion = \"{stale_lock_version}\"\n"
        ),
    );

    let alef_toml = format!(
        "[workspace]\nlanguages = [\"rust\"]\n[[crates]]\nname = \"sample\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = write_file(root, "alef.toml", &alef_toml);
    git_add(
        root,
        &[
            "Cargo.toml",
            "alef.toml",
            "crates/sample/Cargo.toml",
            "crates/sample/src/lib.rs",
            "test_apps/rust/Cargo.toml",
            "test_apps/rust/Cargo.lock",
            "test_apps/rust/src/main.rs",
        ],
    );

    let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = cfg.resolve().expect("resolve config");
    let config = resolved.remove(0);

    {
        let _cwd = CwdGuard::enter(root);
        sync_versions(&config, &alef_toml_path, None, true, true, None).expect("sync_versions ok");
    }

    let lock = std::fs::read_to_string(root.join("test_apps/rust/Cargo.lock")).expect("read blocked lock");
    assert!(
        lock.contains(&format!("version = \"{stale_lock_version}\"")),
        "a lockfile pinning a registry dependency at the version being released must be left \
         byte-for-byte untouched -- cargo cannot resolve it offline before publish: {lock}"
    );

    let checks = validate_versions_run(&config, root, false).expect("validate versions must examine the fixture");
    let blocked = checks
        .iter()
        .find(|check| check.label == "test_apps/rust/Cargo.lock#sample-e2e-rust")
        .expect("the still-stale local package row must still be reported, not silently dropped");
    assert!(
        !blocked.matches,
        "the row genuinely does not match the canonical version yet: {blocked:?}"
    );
    assert_eq!(
        blocked.blocked_on_publish.as_deref(),
        Some(format!("sample@{NEW_VERSION}")).as_deref(),
        "the row must name the release its lockfile is waiting on: {blocked:?}"
    );
    assert!(
        checks_pass(&checks),
        "a check blocked on the pending release must not fail the gate: {checks:?}"
    );
}
