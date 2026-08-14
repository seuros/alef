use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Conventional package roots that hold TypeScript-facing output for the
/// wasm/node targets but are not returned by `package_dir` (which resolves to
/// each target's Rust crate directory). They are swept only on unfiltered runs
/// so orphaned TS artifacts are reclaimed without a per-target directory.
const UNFILTERED_TS_ROOTS: [&str; 2] = ["packages/wasm", "packages/typescript"];

/// Candidate roots for `sweep_orphans` in a `generate` / `all` run.
///
/// The returned paths are candidates only — callers filter out non-existent
/// directories before sweeping (kept out of this function so it stays pure and
/// unit-testable).
///
/// Unfiltered runs (`filtered == false`, i.e. no `--lang`) sweep every
/// configured language's output directory plus the conventional
/// wasm/typescript package roots, so orphans anywhere in the binding tree are
/// reclaimed. The keep set then contains every language's files, so nothing
/// valid is deleted.
///
/// Filtered runs (`--lang <subset>`) must NOT touch output belonging to
/// languages outside the subset. On a filtered run the keep set only contains
/// the requested languages' files, so sweeping another language's root would
/// delete its still-valid generated output (the data-loss bug in #178).
/// Filtered runs therefore restrict roots to the requested languages' own
/// directories and skip the unconditional wasm/typescript roots. Orphans in an
/// unrequested language's tree are reclaimed on the next unfiltered run.
pub fn generate_sweep_roots(
    languages: &[Language],
    filtered: bool,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
) -> Vec<PathBuf> {
    let mut roots: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for &lang in languages {
        roots.insert(base_dir.join(config.package_dir(lang)));
        if let Some(out) = config.output_for(&lang.to_string()) {
            roots.insert(base_dir.join(out));
        }
    }
    if !filtered {
        for root in UNFILTERED_TS_ROOTS {
            roots.insert(base_dir.join(root));
        }
    }
    roots.into_iter().collect()
}

pub fn targeted_e2e_sweep_roots(
    output_paths: &[PathBuf],
    e2e_output_root: &Path,
    snippet_output_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = std::collections::BTreeSet::new();
    for path in output_paths {
        if snippet_output_root.is_some_and(|snippet_root| path.starts_with(snippet_root)) {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(e2e_output_root) {
            let mut components = relative.components();
            let Some(language) = components.next() else {
                continue;
            };
            let Some(owned_subtree) = components.next() else {
                continue;
            };
            if components.next().is_none() {
                continue;
            }
            roots.insert(
                e2e_output_root
                    .join(language.as_os_str())
                    .join(owned_subtree.as_os_str()),
            );
        }
    }
    roots.into_iter().collect()
}

/// Delete alef-generated files under `roots` whose absolute path is not
/// present in `keep`. A file is considered alef-owned only when it has both a
/// recognized Alef marker near its start and an `alef:hash:` line. User code,
/// fixtures, scaffolded manifests, and lockfiles are left untouched.
///
/// This sweeps orphans left behind when categories or fixtures are removed
/// from the generation set (e.g. a category that produced 0 test functions
/// for the current binding surface). Without this pass, those files linger
/// on disk with stale `alef:hash:` headers and `alef verify` reports them
/// as stale forever.
///
/// Empty parent directories left behind after deletion are removed in a
/// best-effort second pass.
pub fn sweep_orphans(
    roots: &[std::path::PathBuf],
    keep: &std::collections::HashSet<std::path::PathBuf>,
) -> anyhow::Result<usize> {
    let mut removed = 0usize;
    let mut touched_dirs: std::collections::BTreeSet<std::path::PathBuf> = std::collections::BTreeSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if file_type.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if matches!(
                        name,
                        ".git"
                            | "target"
                            | "node_modules"
                            | "vendor"
                            | "_build"
                            | "deps"
                            | ".venv"
                            | "venv"
                            | "build"
                            | "dist"
                            | "Pods"
                    ) {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if keep.contains(&path) {
                    continue;
                }
                if !path_is_alef_owned(&path) {
                    continue;
                }
                if let Err(err) = std::fs::remove_file(&path) {
                    debug!("  sweep skip (remove failed): {} ({err})", path.display());
                    continue;
                }
                debug!("  swept orphan: {}", path.display());
                if let Some(parent) = path.parent() {
                    touched_dirs.insert(parent.to_path_buf());
                }
                removed += 1;
            }
        }
    }
    let mut dirs: Vec<_> = touched_dirs.into_iter().collect();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let _ = std::fs::remove_dir(&dir);
    }
    if removed > 0 {
        info!("Swept {removed} orphan generated file(s)");
    }
    Ok(removed)
}

pub fn sweep_manifest_orphans(
    previous_paths: &[PathBuf],
    keep: &std::collections::HashSet<PathBuf>,
    allowed_roots: &[PathBuf],
) -> anyhow::Result<usize> {
    let mut removed = 0;
    for path in previous_paths {
        if keep.contains(path) || !allowed_roots.iter().any(|root| path.starts_with(root)) || !path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        if !content_is_alef_owned(&content) {
            continue;
        }
        std::fs::remove_file(path).with_context(|| format!("failed to remove orphan {}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

/// Collect every alef-headered file under `root` (recursively), skipping
/// dependency / build directories.
///
/// Used by the `all` pipeline to gather existing registry-mode e2e files
/// (`test_apps/`) so their `alef:hash:` lines can be re-stamped after the
/// sources hash changes — without regenerating their content.
pub fn collect_alef_headered_paths(root: &std::path::Path) -> std::collections::HashSet<std::path::PathBuf> {
    let mut paths = std::collections::HashSet::new();
    if !root.exists() {
        return paths;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(
                    name,
                    ".git"
                        | "target"
                        | "node_modules"
                        | "vendor"
                        | "_build"
                        | "deps"
                        | ".venv"
                        | "venv"
                        | "build"
                        | "dist"
                        | "Pods"
                ) {
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() && path_is_alef_owned(&path) {
                paths.insert(path);
            }
        }
    }
    paths
}

fn path_is_alef_owned(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|content| content_is_alef_owned(&content))
}

fn content_is_alef_owned(content: &str) -> bool {
    crate::core::hash::content_has_alef_marker(content) && crate::core::hash::extract_hash(content).is_some()
}

#[cfg(test)]
mod sweep_roots_tests {
    use super::*;

    fn config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: "sample".to_string(),
            ..ResolvedCrateConfig::default()
        }
    }

    /// Regression for #178: a filtered `generate --lang python` must not return
    /// any other language's directory, nor the unconditional wasm/typescript
    /// roots. Sweeping them with a python-only keep set was deleting every other
    /// binding's still-valid output.
    #[test]
    fn filtered_run_excludes_other_languages_and_ts_fallback_roots() {
        let base = Path::new("/base");
        let roots = generate_sweep_roots(&[Language::Python], true, &config(), base);

        assert!(
            roots.contains(&base.join("packages/python")),
            "must keep the requested language"
        );
        assert!(
            !roots.contains(&base.join("packages/ruby")),
            "must not touch an unrequested language"
        );
        assert!(
            !roots.contains(&base.join("packages/wasm")),
            "must not add the wasm fallback root on a filtered run"
        );
        assert!(
            !roots.contains(&base.join("packages/typescript")),
            "must not add the typescript fallback root on a filtered run"
        );
    }

    #[test]
    fn unfiltered_run_includes_all_languages_and_ts_fallback_roots() {
        let base = Path::new("/base");
        let roots = generate_sweep_roots(&[Language::Python, Language::Ruby], false, &config(), base);

        assert!(roots.contains(&base.join("packages/python")));
        assert!(roots.contains(&base.join("packages/ruby")));
        assert!(
            roots.contains(&base.join("packages/wasm")),
            "unfiltered run keeps the wasm fallback root"
        );
        assert!(
            roots.contains(&base.join("packages/typescript")),
            "unfiltered run keeps the typescript fallback root"
        );
    }

    #[test]
    fn manifest_sweep_removes_only_prior_managed_orphans_in_selected_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let selected = dir.path().join("python");
        let other = dir.path().join("ruby");
        std::fs::create_dir_all(&selected).expect("selected root");
        std::fs::create_dir_all(&other).expect("other root");
        let managed = selected.join("orphan.py");
        let handwritten = selected.join("notes.py");
        let unselected = other.join("orphan.rb");
        let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
        std::fs::write(&managed, &hashed).expect("managed");
        std::fs::write(&handwritten, "handwritten\n").expect("handwritten");
        std::fs::write(&unselected, &hashed).expect("unselected");

        let previous = vec![managed.clone(), handwritten.clone(), unselected.clone()];
        let removed =
            sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[selected]).expect("manifest sweep");

        assert_eq!(removed, 1);
        assert!(!managed.exists());
        assert!(handwritten.exists());
        assert!(unselected.exists());
    }

    #[test]
    fn manifest_sweep_preserves_non_header_generator_manifests() {
        let dir = tempfile::tempdir().expect("tempdir");
        let selected = dir.path().join("swift");
        let manifest = selected.join("rust/Cargo.toml");
        std::fs::create_dir_all(manifest.parent().expect("manifest parent")).expect("selected root");
        let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
        let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
        std::fs::write(&manifest, hashed).expect("manifest");

        let previous = vec![manifest.clone()];
        let keep = std::collections::HashSet::from([manifest.clone()]);
        let removed = sweep_manifest_orphans(&previous, &keep, &[selected]).expect("manifest sweep");

        assert_eq!(removed, 0);
        assert!(manifest.exists());
    }

    #[test]
    fn recursive_sweep_and_collection_ignore_hash_only_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let generated_root = dir.path().join("java");
        std::fs::create_dir_all(&generated_root).expect("generated root");
        let legacy_scaffold = generated_root.join("settings.gradle.kts");
        std::fs::write(
            &legacy_scaffold,
            format!("// alef:hash:{}\nrootProject.name = \"sample\"\n", "0".repeat(64)),
        )
        .expect("legacy scaffold");

        let removed = sweep_orphans(std::slice::from_ref(&generated_root), &std::collections::HashSet::new())
            .expect("recursive sweep");
        let collected = collect_alef_headered_paths(&generated_root);

        assert_eq!(removed, 0);
        assert!(legacy_scaffold.exists());
        assert!(!collected.contains(&legacy_scaffold));
    }
}
