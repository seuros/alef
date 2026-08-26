//! Artifact packaging — creates distributable archives for each language.

pub mod c_ffi;
pub mod cli;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod gleam;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod node;
pub mod php;
pub mod python;
pub mod ruby;
pub mod swift;
pub(crate) mod template_env;
pub mod util;
pub mod wasm;
pub mod zig;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A produced package artifact.
#[derive(Debug)]
pub struct PackageArtifact {
    /// Path to the artifact file.
    pub path: PathBuf,
    /// Human-readable artifact name.
    pub name: String,
    /// SHA256 hex digest (if computed).
    pub checksum: Option<String>,
}

/// Create a tar.gz archive from a staging directory.
///
/// The staging directory's basename becomes the single top-level entry inside
/// the archive — so callers whose consumers expect that wrapper (CLI tarballs,
/// FFI tarballs, language SDK archives) get the conventional `dirname/...`
/// layout. For consumers that need the staging contents at the archive root
/// (PHP PIE, which probes the extracted-source root for the extension `.so`),
/// use [`create_tar_gz_flat`] instead.
pub fn create_tar_gz(staging_dir: &Path, output_path: &Path) -> Result<()> {
    let file_name = staging_dir
        .file_name()
        .context("staging dir has no file name")?
        .to_string_lossy();

    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(output_path)
        .arg("-C")
        .arg(staging_dir.parent().unwrap_or(Path::new(".")))
        .arg(file_name.as_ref())
        .status()?;

    if !status.success() {
        anyhow::bail!("tar failed with exit code {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Create a tar.gz archive whose entries are the contents of `staging_dir`,
/// without the wrapping directory.
///
/// PHP PIE's `UnixBuild` probes the extracted-source root for the extension
/// `.so`; if it sees only a single subdirectory it would `unfoldUnarchivedSourcePaths()`,
/// but only when that subdir contains `config.m4` / `config.w32`. Our PIE
/// archive is a precompiled binary with neither, so PIE never unfolds and the
/// install fails with "extension not found". Archive contents directly so the
/// `.so` lands at the archive root.
///
/// Entries are enumerated explicitly rather than passing `.` to `tar`, because
/// `tar czf out.tgz -C dir .` emits a leading `./` directory entry that PIE's
/// Phar-based `TarDownloader` rejects with `Cannot extract ".", internal error`.
/// Passing each top-level entry by name produces a flat archive with no
/// directory entries at all.
pub fn create_tar_gz_flat(staging_dir: &Path, output_path: &Path) -> Result<()> {
    let mut entries: Vec<String> = std::fs::read_dir(staging_dir)
        .with_context(|| format!("reading staging dir {}", staging_dir.display()))?
        .map(|res| {
            res.map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    if entries.is_empty() {
        anyhow::bail!(
            "staging dir {} is empty; refusing to create empty archive",
            staging_dir.display()
        );
    }
    entries.sort();

    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(output_path)
        .arg("-C")
        .arg(staging_dir)
        .args(&entries)
        .status()?;

    if !status.success() {
        anyhow::bail!("tar failed with exit code {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Find a built artifact in the target directory.
///
/// Searches, in order: `target/{triple}/release/`, `target/release/`, then each of those two
/// again with a trailing `deps/`.
///
/// Cargo only copies ("uplifts") an unhashed artifact into `target/{triple}/release/` or
/// `target/release/` directly for a package that was an *explicit* root of that particular
/// `cargo build` invocation (a `-p`/`--manifest-path` target, or a workspace default-member).
/// A crate compiled only because something else path-depends on it — e.g. the `-ffi` crate
/// pulled in by a `-swift`/`-jni` binding crate's own build — lands only in `target/.../deps/`,
/// still unhashed for a `cdylib`/`staticlib` crate type, and never gets uplifted at all. Every
/// caller here wants "the artifact this workspace most recently produced", not "the artifact a
/// specific invocation shape happened to uplift", so `deps/` is checked last rather than skipped:
/// it is the one location cargo unconditionally populates for any crate that was compiled at
/// all, uplifted or not. ~keep
pub fn find_built_artifact(
    workspace_root: &Path,
    target: &crate::publish::platform::RustTarget,
    filename: &str,
) -> Result<PathBuf> {
    let cross_release = workspace_root.join("target").join(&target.triple).join("release");
    let native_release = workspace_root.join("target/release");
    for candidate_dir in [
        cross_release.clone(),
        native_release.clone(),
        cross_release.join("deps"),
        native_release.join("deps"),
    ] {
        let candidate = candidate_dir.join(filename);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "{filename} not found in target/{}/release/, target/release/, or either directory's deps/ subdirectory",
        target.triple
    )
}

#[cfg(test)]
mod find_built_artifact_tests {
    use super::find_built_artifact;
    use crate::publish::platform::RustTarget;

    /// Regression for alef #456's follow-up: a crate compiled only because another crate
    /// path-depends on it (e.g. `-ffi` pulled in by a `-swift`/`-jni` binding crate's own
    /// build) never gets uplifted to `target/release/` -- only `target/release/deps/` is
    /// guaranteed to hold it. Without the `deps/` fallback, every caller of
    /// `find_built_artifact` (FFI staging, Zig/Go/C#/CLI packaging) reports "not found" even
    /// though cargo did compile the artifact this run.
    #[test]
    fn finds_artifact_that_only_landed_in_deps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let deps_dir = root.join("target/release/deps");
        std::fs::create_dir_all(&deps_dir).expect("create deps dir");
        std::fs::write(deps_dir.join("libsample_ffi.so"), b"deps-only-artifact").expect("write fixture");

        let found = find_built_artifact(root, &target, "libsample_ffi.so").expect("must find deps-only artifact");
        assert_eq!(found, deps_dir.join("libsample_ffi.so"));
    }

    /// An uplifted `target/release/` copy must still win over `deps/` when both exist --
    /// `deps/` is a fallback of last resort, not preferred over the primary location.
    #[test]
    fn prefers_uplifted_artifact_over_deps_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let release_dir = root.join("target/release");
        std::fs::create_dir_all(&release_dir).expect("create release dir");
        std::fs::write(release_dir.join("libsample_ffi.so"), b"uplifted").expect("write uplifted fixture");

        let deps_dir = release_dir.join("deps");
        std::fs::create_dir_all(&deps_dir).expect("create deps dir");
        std::fs::write(deps_dir.join("libsample_ffi.so"), b"deps-copy").expect("write deps fixture");

        let found = find_built_artifact(root, &target, "libsample_ffi.so").expect("must find uplifted artifact");
        assert_eq!(found, release_dir.join("libsample_ffi.so"));
    }

    #[test]
    fn still_errors_when_absent_everywhere_including_deps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").expect("parse target");

        let result = find_built_artifact(root, &target, "libsample_ffi.so");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
