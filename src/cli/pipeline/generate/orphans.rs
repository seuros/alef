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

/// Delete alef-emitted files listed in `previous_paths` whose absolute path is
/// no longer present in `keep`, restricted to `allowed_roots`.
///
/// A path is reclaimed by either of two independent routes (see
/// [`path_is_reclaimable`]): content that still carries alef's `alef:hash:`
/// marker (the original, content-based check — unchanged), or membership in the
/// narrow, explicit [`UNMARKABLE_ALEF_MANIFESTS`] allowlist for manifests that
/// structurally cannot carry a marker at all (plain-JSON files such as
/// `composer.json`/`package.json`). For that second route, `previous_paths`
/// membership itself is the ownership evidence: every caller sources it from
/// alef's own cache (`{stage}.manifest` / `{lang}.manifest`), which nothing but
/// alef's own writers ever populate, so reaching this function with a matching
/// path already proves alef wrote it on the run that produced that manifest.
///
/// Reclaiming an unmarkable manifest also reclaims its package-manager-owned
/// lockfile sibling in the same directory (`composer.lock`, `pnpm-lock.yaml`,
/// ...) if present — see [`reclaim_lockfile_siblings`]. Alef never authors a
/// lockfile itself, so a lockfile is *never* reclaimed on its own provenance,
/// only as a cascade of its manifest being reclaimed first.
///
/// This sweeps orphans left behind when categories or fixtures are removed from
/// the generation set, and when a scaffold manifest's emit condition changes
/// (e.g. a co-located/split layout toggle stops emitting a second
/// `composer.json`). Without the unmarkable-manifest route, such files linger on
/// disk forever, since `content_is_alef_owned` can never match a file that never
/// carried a marker in the first place.
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
        if !path_is_reclaimable(path) {
            continue;
        }
        std::fs::remove_file(path).with_context(|| format!("failed to remove orphan {}", path.display()))?;
        removed += 1;
        removed += reclaim_lockfile_siblings(path, keep)?;
    }
    Ok(removed)
}

/// True when `path` has enough ownership evidence to be reclaimed by
/// [`sweep_manifest_orphans`]. See that function's doc for the two routes.
///
/// Route 2 (unmarkable-manifest) intentionally does **not** inspect content, so
/// a hand-edit to e.g. `composer.json` at a path alef has decided to stop
/// writing is reclaimed right along with it — same as any other orphan. This is
/// a deliberate call, not an oversight: a manifest alef *still* writes this run
/// never reaches this function at all (the `keep.contains(path)` check in
/// `sweep_manifest_orphans` filters it out first), so the only files route 2 can
/// ever touch are ones alef's own current-run intent has already disowned. See
/// the accompanying report for the full trade-off discussion. ~keep
fn path_is_reclaimable(path: &Path) -> bool {
    let has_marker = std::fs::read_to_string(path).is_ok_and(|content| content_is_alef_owned(&content));
    has_marker || is_unmarkable_alef_manifest(path)
}

/// Manifest filenames alef scaffolds in a format that cannot carry a marker
/// comment at all (plain JSON: no `#`, no `//`), and therefore emits with
/// `generated_header: false` — see `scaffold_php`'s `composer.json` and
/// `scaffold_node`'s `package.json`. Restricting this list to formats that are
/// *structurally* incapable of the marker — rather than every
/// `generated_header: false` scaffold file — keeps this route from silently
/// widening to cover files that merely opted out of a marker for unrelated
/// reasons (e.g. `pubspec.yaml`, which is YAML and could carry a `#` marker).
/// Extend deliberately, one verified format at a time: check the candidate's
/// actual `generated_header` value and content format in
/// `src/scaffold/languages/*.rs` before adding it here. ~keep
const UNMARKABLE_ALEF_MANIFESTS: &[&str] = &["composer.json", "package.json"];

fn is_unmarkable_alef_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| UNMARKABLE_ALEF_MANIFESTS.contains(&name))
}

/// Manifest filename -> the lockfile name(s) a package manager writes beside it.
/// Alef never authors any of these lockfiles itself (they come from `composer
/// update`, `pnpm install`, `npm install`, `yarn install`, run either by a
/// developer directly or by a `[crates.lint.*]`/`[crates.update.*]` task hook),
/// so a lockfile can never independently satisfy [`path_is_reclaimable`] — it
/// never carries a marker and never appears in `previous_paths` on its own
/// provenance. It is reclaimed *only* as a cascade of its manifest being
/// reclaimed: once alef confirms (by removing `composer.json` here) that it no
/// longer wants that manifest at a given path, a `composer.lock` beside it is
/// installing a manifest that no longer exists, so it goes with it. There is no
/// route by which this module reclaims a lockfile independently of its
/// manifest. ~keep
const MANIFEST_LOCKFILE_SIBLINGS: &[(&str, &[&str])] = &[
    ("composer.json", &["composer.lock"]),
    ("package.json", &["package-lock.json", "pnpm-lock.yaml", "yarn.lock"]),
];

fn reclaim_lockfile_siblings(manifest_path: &Path, keep: &std::collections::HashSet<PathBuf>) -> anyhow::Result<usize> {
    let Some(manifest_name) = manifest_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(0);
    };
    let Some(dir) = manifest_path.parent() else {
        return Ok(0);
    };
    let mut removed = 0;
    for entry in MANIFEST_LOCKFILE_SIBLINGS {
        if entry.0 != manifest_name {
            continue;
        }
        for &lockfile_name in entry.1 {
            let lockfile_path = dir.join(lockfile_name);
            if keep.contains(&lockfile_path) || !lockfile_path.is_file() {
                continue;
            }
            std::fs::remove_file(&lockfile_path)
                .with_context(|| format!("failed to remove orphan lockfile {}", lockfile_path.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Collect every alef-headered file under `root` (recursively), skipping
/// dependency / build directories.
///
/// Used by the `all` pipeline to gather existing registry-mode e2e files
/// (`test_apps/`) so their `alef:hash:` lines can be re-stamped after the
/// sources hash changes — without regenerating their content. Also used by
/// [`super::write::finalize_hashes_sweeping`] as a self-healing safety net:
/// matching on the generated-file **marker** (not on an already-present hash)
/// is deliberate here — a file can be alef-owned and still be missing its
/// `alef:hash:` line (freshly written and not yet finalized, or stripped by
/// an older/interrupted run), and those are exactly the files that need to be
/// found so they can be re-stamped. Matching on `extract_hash(..).is_some()`
/// instead would make an already-unstamped file permanently invisible to this
/// scan, which is the bug this function exists to fix.
pub fn collect_alef_headered_paths(root: &std::path::Path) -> std::collections::HashSet<std::path::PathBuf> {
    fn is_alef_owned(path: &std::path::Path) -> bool {
        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        crate::core::hash::content_has_alef_marker(&content)
    }

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
            } else if ft.is_file() && is_alef_owned(&path) {
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

    /// A manifest alef wrote last run and does not write this run IS reclaimed,
    /// even though `composer.json` never carries a marker or hash — this is the
    /// core "previous-manifest provenance" case from the crawlberg/spikard
    /// orphan reports: a co-located/split layout toggle stops emitting a
    /// `composer.json` copy, and the old copy must not linger forever.
    #[test]
    fn manifest_sweep_reclaims_unmarkable_manifest_absent_from_current_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        let composer_json = package_dir.join("composer.json");
        std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");

        let previous = vec![composer_json.clone()];
        let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir])
            .expect("manifest sweep");

        assert_eq!(removed, 1, "unmarkable manifest absent from this run must be reclaimed");
        assert!(!composer_json.exists());
    }

    /// A manifest alef writes on both runs is NOT touched: it is present in
    /// `keep` (this run's own output), so the `keep.contains(path)` guard must
    /// short-circuit before `path_is_reclaimable` (and therefore before the
    /// unmarkable-manifest allowlist) is ever consulted.
    #[test]
    fn manifest_sweep_preserves_unmarkable_manifest_still_written_this_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        let composer_json = package_dir.join("composer.json");
        std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");

        let previous = vec![composer_json.clone()];
        let keep = std::collections::HashSet::from([composer_json.clone()]);
        let removed = sweep_manifest_orphans(&previous, &keep, &[package_dir]).expect("manifest sweep");

        assert_eq!(removed, 0, "manifest still written this run must survive");
        assert!(composer_json.exists());
    }

    /// Reclaiming an orphaned `composer.json` cascades to its `composer.lock`
    /// sibling in the same directory — the crawlberg concrete instance: a
    /// lockfile whose manifest was removed long ago must not linger forever
    /// either, even though alef never authored the lockfile itself.
    #[test]
    fn manifest_sweep_reclaims_lockfile_sibling_of_reclaimed_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        let composer_json = package_dir.join("composer.json");
        let composer_lock = package_dir.join("composer.lock");
        std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");
        std::fs::write(&composer_lock, "{\n  \"_readme\": []\n}\n").expect("composer.lock");

        let previous = vec![composer_json.clone()];
        let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir])
            .expect("manifest sweep");

        assert_eq!(
            removed, 2,
            "both the manifest and its lockfile sibling must be reclaimed"
        );
        assert!(!composer_json.exists());
        assert!(
            !composer_lock.exists(),
            "orphaned lockfile must be reclaimed alongside its manifest"
        );
    }

    /// THE risk-bounding test: a hand-authored `composer.lock` at a path alef
    /// has never emitted a `composer.json` for must survive `--clean`. This is
    /// the case the design explicitly calls out as capable of real damage — a
    /// predicate that was widened to "any lockfile in a package dir" (instead of
    /// "a lockfile beside a manifest this call just reclaimed by provenance")
    /// would delete it, because `previous_paths` is empty here: alef has no
    /// record of ever writing anything in this directory.
    #[test]
    fn manifest_sweep_preserves_hand_authored_lockfile_alef_never_emitted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        // No composer.json ever existed here — this project was never scaffolded
        // by alef, only a developer ran `composer install` by hand.
        let hand_authored_lock = package_dir.join("composer.lock");
        std::fs::write(&hand_authored_lock, "{\n  \"_readme\": [\"hand rolled\"]\n}\n").expect("hand-authored lock");

        let previous: Vec<PathBuf> = Vec::new();
        let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir])
            .expect("manifest sweep");

        assert_eq!(
            removed, 0,
            "a lockfile with no reclaimed manifest sibling must never be touched"
        );
        assert!(
            hand_authored_lock.exists(),
            "hand-authored lockfile alef never emitted must survive"
        );
    }

    /// Second half of the risk-bounding guarantee: even when a real
    /// `composer.json` exists beside the lockfile, the lockfile survives if the
    /// manifest is still being written this run (`keep`) — the cascade must only
    /// fire when the manifest itself was actually reclaimed, never merely
    /// because a lockfile happens to sit beside *some* manifest.
    #[test]
    fn manifest_sweep_preserves_lockfile_when_its_manifest_is_still_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        let composer_json = package_dir.join("composer.json");
        let composer_lock = package_dir.join("composer.lock");
        std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");
        std::fs::write(&composer_lock, "{\n  \"_readme\": []\n}\n").expect("composer.lock");

        let previous = vec![composer_json.clone()];
        let keep = std::collections::HashSet::from([composer_json.clone()]);
        let removed = sweep_manifest_orphans(&previous, &keep, &[package_dir]).expect("manifest sweep");

        assert_eq!(removed, 0);
        assert!(composer_json.exists());
        assert!(
            composer_lock.exists(),
            "lockfile must survive when its manifest is still written this run"
        );
    }

    /// Documents the chosen behavior for the genuinely contentious case: a user
    /// hand-edits an unmarkable manifest (`composer.json`) at a path alef
    /// previously wrote and has now decided to stop writing. The unmarkable-
    /// manifest route does not inspect content (see `path_is_reclaimable`'s
    /// doc), so the hand-edited file is reclaimed exactly like an untouched one
    /// — deletion wins over preserving the edit, because alef's current-run
    /// intent ("this path is not mine") is unambiguous and the file is already
    /// orphaned relative to that intent.
    #[test]
    fn manifest_sweep_reclaims_user_modified_unmarkable_manifest_at_disowned_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/php");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        let composer_json = package_dir.join("composer.json");
        std::fs::write(
            &composer_json,
            "{\n  \"name\": \"acme/demo\",\n  \"require\": {\n    \"hand-added/package\": \"^9.9\"\n  }\n}\n",
        )
        .expect("hand-edited composer.json");

        let previous = vec![composer_json.clone()];
        let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir])
            .expect("manifest sweep");

        assert_eq!(
            removed, 1,
            "hand-edited content at a disowned path is reclaimed, not preserved"
        );
        assert!(!composer_json.exists());
    }

    /// The unmarkable-manifest allowlist is narrow by construction: a filename
    /// outside [`UNMARKABLE_ALEF_MANIFESTS`] (e.g. `pubspec.yaml`, which opts
    /// out of a marker for unrelated reasons) still requires the original
    /// content-based marker check. This guards against the allowlist silently
    /// growing to swallow every `generated_header: false` scaffold file.
    #[test]
    fn manifest_sweep_does_not_widen_provenance_route_beyond_the_allowlist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let package_dir = dir.path().join("packages/dart");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        let pubspec = package_dir.join("pubspec.yaml");
        std::fs::write(&pubspec, "name: demo\nversion: 1.0.0\n").expect("pubspec.yaml");

        let previous = vec![pubspec.clone()];
        let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir])
            .expect("manifest sweep");

        assert_eq!(
            removed, 0,
            "a filename outside the allowlist must still require the marker check"
        );
        assert!(pubspec.exists());
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
