//! Session fingerprinting: the content-and-configuration digest that keys a session's persistent
//! scratch, its toolchain caches and every snippet cache entry validated against it.
//!
//! Split out of `session` under the repo's file-size cap.

use super::SessionSpec;
use crate::snippets::error::{Error, Result};
use rayon::prelude::*;
use std::path::Path;

/// Directories whose contents are strictly a build tool's own scratch output -- artifacts a
/// deterministic recompile from the source this fingerprint *does* hash reproduces byte-for-byte,
/// with nothing filesystem-resident that isn't already covered. Dropping them changes no
/// fingerprint that should have changed, while a package directory carrying a built `target/`,
/// `dist/`, `Pods/` or `.gradle/` is hundreds of megabytes that were being walked and read in full,
/// per session, per run. ~keep
///
/// `node_modules` and `vendor` are deliberately absent from this list. Both are commonly claimed
/// to be "derived from files that are hashed (lockfiles, manifests)" -- true for an ordinary
/// third-party dependency pinned by version, false for the one entry every binding session cares
/// about most: a locally linked or locally vendored copy of the *consumer's own generated
/// binding*. A `file:`/`link:`/`replace` dependency's resolved content can change with no lockfile
/// line moving at all (same name, same version pin, different bytes on disk), so excluding these
/// two names left every language whose toolchain resolves imports through them -- TypeScript,
/// Node, WASM, PHP's Composer `vendor/`, Ruby's bundler `vendor/` -- unable to detect a changed
/// binding surface at all: `ValidationCache` kept replaying a snippet's last verdict against
/// whatever `node_modules`/`vendor` held the first time the fingerprint was computed, regardless of
/// what a later `alef build` regenerated there. See
/// `node_modules_and_vendor_contents_change_the_fingerprint` below for the regression this
/// closes. ~keep
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
    "obj",
    "target",
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
    use crate::snippets::types::{Language, ValidationLevel};
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

    /// A build tool's own scratch output is deterministically derived from files the fingerprint
    /// already hashes, so reading it cost a full walk of hundreds of megabytes per session per run
    /// and bought nothing. `node_modules` and `vendor` are deliberately not exercised here any
    /// more -- see `node_modules_contents_change_the_fingerprint` below, which asserts the opposite
    /// for exactly those two. ~keep
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

    /// The regression this fix closes: a locally linked/vendored copy of the consumer's own
    /// generated binding resolves through exactly these two directory names in practice --
    /// `node_modules/<package>` for every npm-resolved TypeScript/Node/WASM session (see
    /// `TypeScriptValidator`'s own tests, which build fixture sessions the same way), `vendor/` for
    /// Composer/Bundler. Before this fix both names were in `IGNORED_DIRECTORIES`, so a real content
    /// change to the generated binding underneath either one left `session_fingerprint` -- and
    /// therefore `ValidationCache`'s key, which folds this fingerprint in -- completely unchanged.
    /// A snippet that had already cached a `Pass` against the old binding surface kept replaying
    /// that verdict forever, regardless of what the binding surface actually declared afterward.
    /// ~keep
    #[test]
    fn node_modules_and_vendor_contents_change_the_fingerprint() {
        for package_root in ["node_modules/sample-binding", "vendor/sample-binding"] {
            let directory = tempfile::tempdir().expect("temp directory");
            let package = directory.path().join(package_root);
            std::fs::create_dir_all(&package).expect("linked package directory");
            std::fs::write(package.join("index.d.ts"), "export declare const maxChars: number;")
                .expect("original binding surface");
            let spec = fingerprint_spec(directory.path());
            let before = session_fingerprint(&spec).expect("fingerprint before the binding surface changed");

            std::fs::write(
                package.join("index.d.ts"),
                "export declare const maxCharacters: number;",
            )
            .expect("regenerated binding surface");
            let after = session_fingerprint(&spec).expect("fingerprint after the binding surface changed");

            assert_ne!(
                before, after,
                "a content change under {package_root} must change the session fingerprint, not be \
                 silently ignored"
            );
        }
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

    fn cached_snippet(code: &str) -> crate::snippets::types::Snippet {
        let path = std::path::PathBuf::from("example.md");
        crate::snippets::types::Snippet {
            id: None,
            path: path.clone(),
            language: Language::TypeScript,
            title: None,
            code: code.to_string(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: crate::snippets::types::SnippetMetadata::default(),
            source_origin: crate::snippets::types::SourceOrigin {
                path,
                line: 1,
                block_index: 0,
            },
        }
    }

    fn passing_result(snippet: &crate::snippets::types::Snippet) -> crate::snippets::types::ValidationResult {
        crate::snippets::types::ValidationResult {
            snippet: snippet.clone(),
            status: crate::snippets::types::SnippetStatus::Pass,
            level: ValidationLevel::Compile,
            requested_level: ValidationLevel::Compile,
            effective_level: ValidationLevel::Compile,
            message: None,
            duration_ms: 1,
            capability_capped: false,
            downgrade_reason: None,
            unresolved_dependency: false,
        }
    }

    /// The end-to-end regression this fix closes, chained through the real production types
    /// instead of asserting on the fingerprint alone: a snippet that already has a cached `Pass`
    /// from validating against binding surface A must miss the cache -- forcing the runner to
    /// invoke the validator again, not replay the stale verdict -- once the linked binding package
    /// changes to surface B. `ValidationCache::load` returning `None` is exactly the signal
    /// `runner::cached_result` reads to fall through to a real validator invocation (see
    /// `runner.rs::validate_one`), so a `None` here is not a proxy for re-validation, it is the
    /// mechanism that causes it.
    ///
    /// Before this fix, `node_modules` was excluded from `IGNORED_DIRECTORIES`: `before_fingerprint`
    /// and `after_fingerprint` were identical despite the package's content changing, so the second
    /// `load` below returned `Some(passing)` instead of `None` -- a stale cached `Pass` served for a
    /// snippet that had never been re-validated against the new binding surface at all. ~keep
    #[test]
    fn a_cached_pass_misses_once_the_linked_binding_package_changes() {
        let directory = tempfile::tempdir().expect("temp directory");
        let package = directory.path().join("node_modules/sample-binding");
        std::fs::create_dir_all(&package).expect("linked package directory");
        std::fs::write(
            package.join("index.d.ts"),
            "export declare function chunk(maxChars: number): void;",
        )
        .expect("original binding surface");
        let spec = fingerprint_spec(directory.path());
        let cache = crate::snippets::cache::ValidationCache::new(directory.path().join(".alef/snippets"));
        let snippet = cached_snippet("chunk(10)");
        let passing = passing_result(&snippet);

        let before_fingerprint = session_fingerprint(&spec).expect("fingerprint before the binding surface changed");
        cache
            .store(
                &snippet,
                ValidationLevel::Compile,
                Some(before_fingerprint.as_str()),
                &passing,
            )
            .expect("store the passing result");

        // Negative control: nothing about the snippet or the binding surface changed, so the exact
        // same fingerprint must still be a cache hit. A "fix" that disabled caching outright (always
        // returning `None` from `load`, or skipping `store` entirely) would make every run pay for a
        // full re-validation regardless of whether anything changed -- turning a `changed_only` run
        // across thousands of snippets back into a full run every time. This assertion is what would
        // fail if someone "fixed" the bug that way. ~keep
        assert_eq!(
            cache
                .load(&snippet, ValidationLevel::Compile, Some(before_fingerprint.as_str()))
                .map(|result| result.status),
            Some(crate::snippets::types::SnippetStatus::Pass),
            "an unchanged snippet against an unchanged binding surface must still hit the cache"
        );

        std::fs::write(
            package.join("index.d.ts"),
            "export declare function chunk(maxCharacters: number): void;",
        )
        .expect("regenerated binding surface");
        let after_fingerprint = session_fingerprint(&spec).expect("fingerprint after the binding surface changed");

        assert!(
            cache
                .load(&snippet, ValidationLevel::Compile, Some(after_fingerprint.as_str()))
                .is_none(),
            "a cached Pass must not survive a change to the linked binding package it was validated \
             against"
        );
    }
}
