//! Version sync must not rewrite files git ignores.
//!
//! Every rewrite in `sync_versions` that finds its target by expanding a glob used to write
//! wherever the glob reached, and build tools mirror real source trees into disposable staging
//! directories that match those globs verbatim: setuptools copies `packages/python/**` into
//! `packages/python/build/lib/`, gem packaging stages into `tmp/`. Bumping a version there
//! produces an edit no review sees and the next clean destroys, while leaving it ambiguous
//! whether the tracked original was bumped at all.
//!
//! Every test here therefore asserts in two directions: the ignored copy is *not* written **and**
//! the real file *is*. A filter that refused everything would satisfy the first assertion alone,
//! and "wrote nothing" is the failure mode a version-sync gate is most likely to regress into. ~keep

use super::*;
use crate::core::config::NewAlefConfig;
use crate::test_support::{CwdGuard, git_add, git_init, write_file};
use std::path::Path;

const OLD_VERSION: &str = "1.2.2";
const NEW_VERSION: &str = "1.2.3";

fn csproj(version: &str) -> String {
    format!("<Project>\n  <PropertyGroup>\n    <Version>{version}</Version>\n  </PropertyGroup>\n</Project>\n")
}

fn python_init(version: &str) -> String {
    format!("__version__ = \"{version}\"\n")
}

/// Build a work tree at `root` whose `Cargo.toml` declares `NEW_VERSION`, with an `alef.toml`
/// covering `languages`, and return the resolved config plus its config path.
fn workspace(root: &Path, languages: &str) -> (crate::core::config::ResolvedCrateConfig, std::path::PathBuf) {
    write_file(
        root,
        "Cargo.toml",
        &format!("[workspace.package]\nversion = \"{NEW_VERSION}\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n"),
    );
    let alef_toml = format!(
        "[workspace]\nlanguages = [{languages}]\n[[crates]]\nname = \"example\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = write_file(root, "alef.toml", &alef_toml);
    let config: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = config.resolve().expect("resolve config");
    (resolved.remove(0), alef_toml_path)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read fixture file back")
}

/// The two globs that actually reach a real build tool's staging tree — `packages/csharp/**`
/// and `packages/python/**` — must skip the ignored copy and still bump the tracked original.
#[test]
fn sync_versions_skips_git_ignored_staging_and_still_bumps_the_tracked_original() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);
    write_file(root, ".gitignore", "tmp/\nbuild/\n");

    let tracked_csproj = write_file(root, "packages/csharp/Sample/Sample.csproj", &csproj(OLD_VERSION));
    let staged_csproj = write_file(root, "packages/csharp/tmp/stage/Sample.csproj", &csproj(OLD_VERSION));
    let tracked_init = write_file(root, "packages/python/sample/__init__.py", &python_init(OLD_VERSION));
    let staged_init = write_file(
        root,
        "packages/python/build/lib/sample/__init__.py",
        &python_init(OLD_VERSION),
    );

    let (config, config_path) = workspace(root, "\"csharp\", \"python\"");
    git_add(
        root,
        &[
            ".gitignore",
            "Cargo.toml",
            "alef.toml",
            "packages/csharp/Sample/Sample.csproj",
            "packages/python/sample/__init__.py",
        ],
    );

    {
        let _cwd = CwdGuard::enter(root);
        sync_versions(&config, &config_path, None, true, true, None).expect("sync_versions ok");
    }

    assert!(
        read(&tracked_csproj).contains(&format!("<Version>{NEW_VERSION}</Version>")),
        "the tracked csproj must still be bumped, or this test passes by writing nothing"
    );
    assert!(
        read(&tracked_init).contains(&format!("__version__ = \"{NEW_VERSION}\"")),
        "the tracked __init__.py must still be bumped, or this test passes by writing nothing"
    );
    assert_eq!(
        read(&staged_csproj),
        csproj(OLD_VERSION),
        "a csproj under a gitignored staging directory must be left byte-for-byte alone"
    );
    assert_eq!(
        read(&staged_init),
        python_init(OLD_VERSION),
        "the setuptools mirror under packages/python/build/lib must be left byte-for-byte alone"
    );
}

/// Ignored-ness, not tracked-ness, is the gate on the write side.
///
/// `alef generate` emits files that are legitimately untracked until the consumer commits them,
/// and the version sync that runs right afterwards must still stamp them. A tracked-ness gate
/// would silently ship those files with a stale version — the exact failure this whole change is
/// meant to prevent, only inverted. ~keep
#[test]
fn sync_versions_still_bumps_an_untracked_file_git_does_not_ignore() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);
    write_file(root, ".gitignore", "tmp/\n");

    let untracked = write_file(root, "packages/csharp/Fresh/Fresh.csproj", &csproj(OLD_VERSION));
    let (config, config_path) = workspace(root, "\"csharp\"");
    git_add(root, &[".gitignore", "Cargo.toml", "alef.toml"]);

    {
        let _cwd = CwdGuard::enter(root);
        sync_versions(&config, &config_path, None, true, true, None).expect("sync_versions ok");
    }

    assert!(
        read(&untracked).contains(&format!("<Version>{NEW_VERSION}</Version>")),
        "freshly generated output the consumer has not committed yet must still be bumped"
    );
}

/// `sync.extra_paths` is consumer-supplied and otherwise entirely unconstrained: whatever the
/// pattern matches gets rewritten. It is the one rewrite surface where a `**` wide enough to
/// reach a staging tree is the consumer's own doing, so it needs the same gate as alef's globs.
#[test]
fn sync_versions_extra_paths_does_not_reach_git_ignored_matches() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    git_init(root);
    write_file(root, ".gitignore", "tmp/\n");

    let tracked = write_file(root, "extra/lib/version.rb", &format!("VERSION = \"{OLD_VERSION}\"\n"));
    let staged = write_file(root, "extra/tmp/version.rb", &format!("VERSION = \"{OLD_VERSION}\"\n"));

    write_file(
        root,
        "Cargo.toml",
        &format!("[workspace.package]\nversion = \"{NEW_VERSION}\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n"),
    );
    let alef_toml = format!(
        "[workspace]\nlanguages = [\"ruby\"]\n\n[workspace.sync]\nextra_paths = [\"extra/*/version.rb\"]\n\n\
         [[crates]]\nname = \"example\"\nsources = []\nversion_from = \"{}\"\n",
        root.join("Cargo.toml").display().to_string().replace('\\', "/")
    );
    let alef_toml_path = write_file(root, "alef.toml", &alef_toml);
    let parsed: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
    let mut resolved = parsed.resolve().expect("resolve config");
    let config = resolved.remove(0);
    assert!(
        config
            .sync
            .as_ref()
            .is_some_and(|sync| sync.extra_paths.iter().any(|p| p == "extra/*/version.rb")),
        "the fixture must actually configure extra_paths, or the gate under test is never reached"
    );
    git_add(root, &[".gitignore", "Cargo.toml", "alef.toml", "extra/lib/version.rb"]);

    {
        let _cwd = CwdGuard::enter(root);
        sync_versions(&config, &alef_toml_path, None, true, true, None).expect("sync_versions ok");
    }

    assert!(
        read(&tracked).contains(NEW_VERSION),
        "the tracked extra_paths match must still be rewritten, or this test proves nothing"
    );
    assert_eq!(
        read(&staged),
        format!("VERSION = \"{OLD_VERSION}\"\n"),
        "an extra_paths match under a gitignored directory must be left byte-for-byte alone"
    );
}

/// Outside a git work tree the filter must degrade to the unfiltered walk it replaces. Refusing
/// to write anything when `git` cannot answer would break every consumer who runs `alef` from an
/// exported tarball. ~keep
#[test]
fn sync_versions_without_a_git_work_tree_writes_everything_it_used_to() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    let staged = write_file(root, "packages/csharp/tmp/stage/Sample.csproj", &csproj(OLD_VERSION));
    let (config, config_path) = workspace(root, "\"csharp\"");

    {
        let _cwd = CwdGuard::enter(root);
        sync_versions(&config, &config_path, None, true, true, None).expect("sync_versions ok");
    }

    assert!(
        read(&staged).contains(&format!("<Version>{NEW_VERSION}</Version>")),
        "with no git available the filter must permit everything, not silently write nothing"
    );
}
