//! Per-language answers to one question: which build artifacts does this session's manifest point
//! its toolchain at that are not on disk yet?
//!
//! Held in one module rather than in each validator because the *rule* is shared even though the
//! parsing is not — every probe here reads a path the session's own manifest names, resolves it,
//! and stats it. Nothing here guesses a conventional location, infers from config shape, or
//! encodes "this language usually needs a build": alef already removed a static
//! `enforce_build_dependency` gate that did exactly that and bailed on languages whose validators
//! need no build at all. A probe that cannot answer with evidence answers "nothing missing", and
//! the language keeps validating exactly as it did before.
//!
//! Callers are the `SnippetValidator::missing_session_artifacts` implementations; the runner reads
//! them once per session in `runner::artifact_preflight`. ~keep

use crate::snippets::session::ValidationSession;
use std::path::{Path, PathBuf};

/// The declaration file a Node/TypeScript package manifest publishes, when the manifest names one
/// and it has not been emitted yet.
///
/// `TypeScriptValidator::package_overlay` maps the session package's own name onto this exact path
/// in the overlay `tsconfig.json`'s `compilerOptions.paths`, so when the file is absent every
/// snippet that imports the package gets the same `Cannot find module` from `tsc` — one fact, N
/// identical diagnostics.
///
/// Two manifests reach this and only one makes a claim. A manifest carrying `compilerOptions` is a
/// `tsconfig.json`, and the validator extends it rather than mapping a package: it names no build
/// output, so this returns nothing. A `package.json` with no `types`/`typings` entry likewise
/// declares no artifact — the overlay falls back to the package root, which exists by construction
/// (the manifest is inside it). ~keep
pub(super) fn missing_typescript_declaration(session: &ValidationSession) -> Vec<PathBuf> {
    let Some(manifest) = session.manifest.as_deref() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(manifest) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    if value.get("compilerOptions").is_some() {
        return Vec::new();
    }
    let Some(entry) = value
        .get("types")
        .or_else(|| value.get("typings"))
        .and_then(serde_json::Value::as_str)
    else {
        return Vec::new();
    };
    let declaration = manifest.parent().unwrap_or_else(|| Path::new(".")).join(entry);
    if declaration.exists() {
        Vec::new()
    } else {
        vec![declaration]
    }
}

/// The FFI library a scaffolded `build.zig` links, when it is on neither the release nor the debug
/// search path. Delegates to [`super::zig::manifest::unresolvable_ffi_library`] so the preflight
/// and the `-Dffi_path` override that runs during validation can never disagree about which
/// profile actually holds the library. ~keep
pub(super) fn missing_zig_ffi_library(session: &ValidationSession) -> Vec<PathBuf> {
    let Some(manifest) = session.manifest.as_deref() else {
        return Vec::new();
    };
    super::zig::manifest::unresolvable_ffi_library(manifest)
        .ok()
        .flatten()
        .into_iter()
        .collect()
}

/// `${SRCDIR}`-relative library search directories a generated cgo package declares that do not
/// exist on disk.
///
/// Deliberately a check on the *directory*, not on the `-l<name>` entries beside it. A cgo
/// directive mixes the package's own built library with ordinary system libraries (`-lm`,
/// `-lpthread`), and nothing in the directive says which is which — probing names would flag a
/// system library the linker resolves from its own search path, a false skip of a corpus that
/// would have validated fine. A `${SRCDIR}`-rooted directory carries no such ambiguity: it points
/// inside the consumer's own tree at a path their build fills, so its absence is direct evidence
/// that the build has not run, and `go build` will fail to link for the package's own library
/// whatever else is on the line.
///
/// Returns nothing when the directory is present but empty — a false negative costs one ordinary
/// validation pass, while a false positive silently stops checking. ~keep
pub(super) fn missing_go_library_directories(session: &ValidationSession) -> Vec<PathBuf> {
    let project = session
        .manifest
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(&session.working_directory);
    let Ok(entries) = std::fs::read_dir(project) else {
        return Vec::new();
    };
    let mut missing = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "go") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for directory in cgo_source_relative_library_directories(&source, project) {
            if !directory.is_dir() && !missing.contains(&directory) {
                missing.push(directory);
            }
        }
    }
    missing.sort();
    missing
}

/// The `-L` arguments of every `#cgo ... LDFLAGS:` directive in `source` that are written relative
/// to `${SRCDIR}`, resolved against `project`. Absolute and bare-relative `-L` paths are ignored:
/// only a `${SRCDIR}` root proves the directory is meant to live inside the package's own tree.
fn cgo_source_relative_library_directories(source: &str, project: &Path) -> Vec<PathBuf> {
    const SOURCE_DIRECTORY: &str = "${SRCDIR}/";

    let mut directories = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let Some(directive) = trimmed.strip_prefix("//") else {
            continue;
        };
        let directive = directive.trim();
        if !directive.starts_with("#cgo ") || !directive.contains("LDFLAGS:") {
            continue;
        }
        for argument in directive.split_whitespace() {
            let Some(path) = argument.strip_prefix("-L") else {
                continue;
            };
            let Some(relative) = path.strip_prefix(SOURCE_DIRECTORY) else {
                continue;
            };
            directories.push(normalized(&project.join(relative)));
        }
    }
    directories
}

/// Resolves `..` components lexically so a `${SRCDIR}/../../target/release` directive is probed as
/// the directory it actually names. `Path::exists` follows `..` fine on its own, but the path is
/// also reported to the operator, and `packages/go/../../target/release` is not a path anyone can
/// act on. ~keep
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::Language;
    use std::collections::BTreeMap;

    fn session(language: Language, working_directory: PathBuf, manifest: Option<PathBuf>) -> ValidationSession {
        ValidationSession {
            language,
            working_directory,
            manifest,
            fingerprint: "fingerprint".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn a_declared_typescript_declaration_file_that_was_never_emitted_is_reported_missing() {
        let package = tempfile::tempdir().expect("package directory");
        let manifest = package.path().join("package.json");
        std::fs::write(&manifest, r#"{"name":"sample-binding","types":"dist/index.d.ts"}"#).expect("manifest");

        let missing = missing_typescript_declaration(&session(
            Language::TypeScript,
            package.path().to_path_buf(),
            Some(manifest),
        ));

        assert_eq!(missing, vec![package.path().join("dist/index.d.ts")]);
    }

    /// The control that keeps the probe from being a blanket skip: the artifact is on disk, so the
    /// session is satisfiable and validation must proceed exactly as before. ~keep
    #[test]
    fn a_typescript_package_whose_declaration_exists_reports_nothing_missing() {
        let package = tempfile::tempdir().expect("package directory");
        let manifest = package.path().join("package.json");
        std::fs::write(&manifest, r#"{"name":"sample-binding","types":"index.d.ts"}"#).expect("manifest");
        std::fs::write(package.path().join("index.d.ts"), "export {};").expect("declaration");

        let missing = missing_typescript_declaration(&session(
            Language::TypeScript,
            package.path().to_path_buf(),
            Some(manifest),
        ));

        assert!(missing.is_empty(), "{missing:?}");
    }

    /// A `tsconfig.json` session manifest declares no build output at all -- the validator extends
    /// it rather than mapping a package onto a declaration file -- so it must never be read as an
    /// unsatisfiable session. ~keep
    #[test]
    fn a_tsconfig_session_manifest_claims_no_artifact() {
        let project = tempfile::tempdir().expect("project directory");
        let manifest = project.path().join("tsconfig.json");
        std::fs::write(&manifest, r#"{"compilerOptions":{"strict":true}}"#).expect("manifest");

        let missing = missing_typescript_declaration(&session(
            Language::TypeScript,
            project.path().to_path_buf(),
            Some(manifest),
        ));

        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn a_cgo_search_directory_the_build_never_produced_is_reported_missing() {
        let root = tempfile::tempdir().expect("root directory");
        let project = root.path().join("packages/go");
        std::fs::create_dir_all(&project).expect("project directory");
        std::fs::write(
            project.join("bindings.go"),
            "package sample\n\n// #cgo LDFLAGS: -L${SRCDIR}/../../target/release -lsample_ffi -lm\nimport \"C\"\n",
        )
        .expect("go source");

        let missing =
            missing_go_library_directories(&session(Language::Go, project.clone(), Some(project.join("go.mod"))));

        assert_eq!(missing, vec![root.path().join("target/release")]);
    }

    /// The control for the Go probe, and the reason it reads directories rather than `-l` names:
    /// `-lm` is a system library no build produces, and a probe that flagged it would skip a
    /// corpus that validates fine. ~keep
    #[test]
    fn a_cgo_directive_whose_search_directory_exists_reports_nothing_missing() {
        let root = tempfile::tempdir().expect("root directory");
        let project = root.path().join("packages/go");
        std::fs::create_dir_all(&project).expect("project directory");
        std::fs::create_dir_all(root.path().join("target/release")).expect("build directory");
        std::fs::write(
            project.join("bindings.go"),
            "package sample\n\n// #cgo LDFLAGS: -L${SRCDIR}/../../target/release -lsample_ffi -lm\nimport \"C\"\n",
        )
        .expect("go source");

        let missing =
            missing_go_library_directories(&session(Language::Go, project.clone(), Some(project.join("go.mod"))));

        assert!(missing.is_empty(), "{missing:?}");
    }

    /// An absolute or bare-relative `-L` says nothing about the consumer's own build tree -- it can
    /// name a system or vendored prefix that is legitimately absent on this host -- so only a
    /// `${SRCDIR}` root is evidence. ~keep
    #[test]
    fn a_cgo_search_directory_outside_the_package_tree_is_never_probed() {
        let project = tempfile::tempdir().expect("project directory");
        std::fs::write(
            project.path().join("bindings.go"),
            "package sample\n\n// #cgo LDFLAGS: -L/opt/vendor/lib -Lrelative/lib -lsample_ffi\nimport \"C\"\n",
        )
        .expect("go source");

        let missing = missing_go_library_directories(&session(
            Language::Go,
            project.path().to_path_buf(),
            Some(project.path().join("go.mod")),
        ));

        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn a_reported_directory_carries_no_parent_components_an_operator_cannot_act_on() {
        let root = tempfile::tempdir().expect("root directory");
        let project = root.path().join("packages/go");

        let directories = cgo_source_relative_library_directories(
            "// #cgo LDFLAGS: -L${SRCDIR}/../../target/release -lsample_ffi",
            &project,
        );

        assert_eq!(directories, vec![root.path().join("target/release")]);
    }
}
