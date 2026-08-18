//! Session fingerprinting: the content-and-configuration digest that keys a session's persistent
//! scratch, its toolchain caches and every snippet cache entry validated against it.
//!
//! Split out of `session` under the repo's file-size cap.

use super::SessionSpec;
use crate::snippets::error::{Error, Result};
use rayon::prelude::*;
use std::path::Path;

/// Directories whose contents are a build tool's own output or a vendored dependency tree, never
/// an input the fingerprint needs to see. Everything here is derived from files that *are* hashed
/// (sources, lockfiles, manifests), so dropping them changes no fingerprint that should have
/// changed — while a package directory carrying a built `target/`, `dist/`, `Pods/` or
/// `.gradle/` is hundreds of megabytes that were being walked and read in full, per session, per
/// run. ~keep
const IGNORED_DIRECTORIES: &[&str] = &[
    ".alef",
    ".dart_tool",
    ".git",
    ".gradle",
    ".next",
    ".pytest_cache",
    ".venv",
    ".zig-cache",
    ".zig-global-cache",
    "Carthage",
    "Pods",
    "__pycache__",
    "_build",
    "bin",
    "build",
    "dist",
    "node_modules",
    "obj",
    "target",
    "vendor",
];

pub(super) fn session_fingerprint(spec: &SessionSpec) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    hash_specification(&mut hasher, spec);
    for digest in working_tree_digests(spec)? {
        hasher.update(digest.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// The configuration half of the fingerprint: everything that distinguishes two sessions pointed
/// at the same working tree.
fn hash_specification(hasher: &mut blake3::Hasher, spec: &SessionSpec) {
    hasher.update(spec.working_directory.to_string_lossy().as_bytes());
    if let Some(manifest) = &spec.manifest {
        hasher.update(manifest.to_string_lossy().as_bytes());
    }
    for command in &spec.before {
        hasher.update(command.as_bytes());
    }
    for (name, value) in &spec.env {
        hasher.update(name.as_bytes());
        hasher.update(value.as_bytes());
    }
    for path in &spec.include_paths {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    for feature in &spec.rust_features {
        hasher.update(feature.as_bytes());
    }
    for (name, dependency) in &spec.rust_dependencies {
        hasher.update(name.as_bytes());
        hasher.update(dependency.version.as_bytes());
        hasher.update(&[u8::from(dependency.default_features)]);
        for feature in &dependency.features {
            hasher.update(feature.as_bytes());
        }
    }
}

/// One digest per file in the working tree, hashed concurrently but returned in relative-path
/// order.
///
/// The sort has to happen before the digests are folded into the session hasher, and it has to be
/// on the *relative* path: a fingerprint that varies between two runs over an unchanged tree
/// silently invalidates every cache entry keyed on it, and `walkdir` gives no ordering guarantee
/// across filesystems. Hashing each file into its own digest first is what lets the read-and-hash
/// step run in parallel while the fold stays ordered. ~keep
fn working_tree_digests(spec: &SessionSpec) -> Result<Vec<blake3::Hash>> {
    let mut files = walkdir::WalkDir::new(&spec.working_directory)
        .into_iter()
        .filter_entry(|entry| {
            !entry.file_type().is_dir() || !IGNORED_DIRECTORIES.contains(&entry.file_name().to_string_lossy().as_ref())
        })
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let path = entry.into_path();
            let relative = path
                .strip_prefix(&spec.working_directory)
                .unwrap_or(&path)
                .to_path_buf();
            (relative, path)
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
        .into_par_iter()
        .map(|(relative, path)| hash_working_tree_file(&relative, &path))
        .collect()
}

fn hash_working_tree_file(relative: &Path, path: &Path) -> Result<blake3::Hash> {
    let content = std::fs::read(path)
        .map_err(|error| Error::Other(format!("hashing snippet session input {}: {error}", path.display())))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(relative.to_string_lossy().as_bytes());
    hasher.update(&content);
    Ok(hasher.finalize())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::Language;
    use std::collections::BTreeMap;

    fn fingerprint_spec(working_directory: &Path) -> SessionSpec {
        SessionSpec {
            language: Language::TypeScript,
            working_directory: working_directory.to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    /// The fingerprint keys the session scratch directory *and* every validation cache entry, so a
    /// digest that varies between two runs over an unchanged tree invalidates the whole cache
    /// silently and rebuilds everything. Hashing files concurrently only stays safe while the fold
    /// order is pinned to the relative path, which `walkdir` does not guarantee on its own. ~keep
    #[test]
    fn the_fingerprint_is_stable_across_runs_and_tracks_source_changes() {
        let directory = tempfile::tempdir().expect("temp directory");
        std::fs::create_dir_all(directory.path().join("src/deep")).expect("source tree");
        for name in ["src/a.ts", "src/b.ts", "src/deep/c.ts", "package.json"] {
            std::fs::write(directory.path().join(name), format!("content of {name}")).expect("source file");
        }
        let spec = fingerprint_spec(directory.path());

        let first = session_fingerprint(&spec).expect("first fingerprint");
        let second = session_fingerprint(&spec).expect("second fingerprint");
        assert_eq!(first, second);

        std::fs::write(directory.path().join("src/b.ts"), "changed").expect("changed source");
        let changed = session_fingerprint(&spec).expect("changed fingerprint");

        assert_ne!(first, changed);
    }

    /// Build output and vendored dependency trees are derived from files the fingerprint already
    /// hashes, so reading them cost a full walk of hundreds of megabytes per session per run and
    /// bought nothing. ~keep
    #[test]
    fn build_output_directories_are_excluded_from_the_fingerprint() {
        let directory = tempfile::tempdir().expect("temp directory");
        std::fs::write(directory.path().join("index.ts"), "export const value = 1;").expect("source file");
        let spec = fingerprint_spec(directory.path());
        let baseline = session_fingerprint(&spec).expect("baseline fingerprint");

        for ignored in IGNORED_DIRECTORIES {
            let artifacts = directory.path().join(ignored);
            std::fs::create_dir_all(&artifacts).expect("artifact directory");
            std::fs::write(artifacts.join("artifact.bin"), ignored.as_bytes()).expect("build artifact");
        }

        assert_eq!(session_fingerprint(&spec).expect("fingerprint after build"), baseline);
    }

    #[test]
    fn include_paths_contribute_to_the_session_fingerprint() {
        let directory = tempfile::tempdir().expect("temp directory");
        let base = SessionSpec {
            language: Language::C,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            before: Vec::new(),
            env: BTreeMap::new(),
            include_paths: vec![directory.path().join("include")],
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };
        let mut changed = base.clone();
        changed.include_paths = vec![directory.path().join("vendor/include")];

        assert_ne!(
            session_fingerprint(&base).expect("base fingerprint"),
            session_fingerprint(&changed).expect("changed fingerprint")
        );
    }

}
