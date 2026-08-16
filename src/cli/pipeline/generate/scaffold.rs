use super::normalization::normalize_content;
use super::write::apply_shebang_chmod;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use base64::Engine;
use std::path::Path;
use tracing::{debug, warn};

/// Generate scaffold files for given languages.
///
/// After the built-in scaffold generators run, each registered extension gets a
/// chance to rewrite the scaffold file set per language via
/// [`crate::core::extension::Extension::transform_scaffold_files`] — for example
/// to wire an ergonomic entry module into a package `main`/wrapper or to add
/// runtime dependencies to a manifest. Extensions receive their
/// `[extensions.<name>]` config from `config_path` (`alef.toml`).
pub fn scaffold(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    config_path: &Path,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = crate::scaffold::scaffold(api, config, languages)?;
    crate::with_extensions(|exts| {
        let env = crate::core::template_env::TemplateEnv::new();
        for ext in exts {
            let raw = crate::core::extension::read_extension_config(config_path, ext.name())
                .with_context(|| format!("extension `{}`: failed to read config from alef.toml", ext.name()))?;
            let cfg = ext
                .parse_config(raw.as_ref())
                .with_context(|| format!("extension `{}`: failed to parse config", ext.name()))?;
            for &language in languages {
                ext.transform_scaffold_files(api, &cfg, language, &mut files, &env)
                    .with_context(|| {
                        format!(
                            "extension `{}`: transform_scaffold_files({language}) failed",
                            ext.name()
                        )
                    })?;
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(files)
}

/// Generate README files for given languages.
pub fn readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    crate::readme::generate_readmes(api, config, languages)
}

/// Write standalone generated files (not grouped by language) to disk.
///
/// Scaffold files are create-only by default: if the target file already exists
/// on disk it is left untouched so that user customisations are preserved.
/// Pass `overwrite = true` (e.g. via `--clean`) to force-write all files.
///
/// Files that carry the alef header marker (regenerated bindings, READMEs)
/// will receive their `alef:hash:` line later via [`super::write::finalize_hashes`] —
/// scaffold files without the marker (Cargo.toml templates, composer.json,
/// gemspec) pass through unchanged.
pub fn write_scaffold_files(files: &[GeneratedFile], base_dir: &Path) -> anyhow::Result<usize> {
    write_scaffold_files_with_overwrite(files, base_dir, false)
}

/// Reconcile generated TOML manifests needed by generated bindings.
///
/// Existing files are eligible only when their embedded marker proves Alef owns
/// them. Missing manifests may be created because the scaffold declaration itself
/// marks them as generated; handwritten manifests and non-manifest scaffold files
/// remain outside this generate-stage repair. ~keep
pub fn reconcile_managed_scaffold_manifests(
    files: &[GeneratedFile],
    base_dir: &Path,
) -> anyhow::Result<super::write::WriteReport> {
    let mut manifests = Vec::new();
    for file in files
        .iter()
        .filter(|file| file.generated_header && file.path.extension().is_some_and(|extension| extension == "toml"))
    {
        let path = base_dir.join(&file.path);
        if !path.exists() {
            manifests.push(file.clone());
            continue;
        }
        let content =
            std::fs::read_to_string(&path).with_context(|| format!("failed to read existing {}", path.display()))?;
        if crate::core::hash::content_has_alef_marker(&content) {
            manifests.push(file.clone());
        }
    }
    write_scaffold_files_report(&manifests, base_dir, false)
}

/// Like [`write_scaffold_files`] but with an explicit `overwrite` flag.
///
/// Files marked `generated_header: true` are always overwritten regardless of the
/// flag: these are fully alef-managed manifests (Cargo.toml, gemspec, composer.json)
/// whose dependency lists are derived from `[workspace.languages]`, `[crates.*]`,
/// and the active adapter set. Skipping them on regen means newly added streaming
/// adapters or trait bridges never get their conditional deps (futures-util,
/// futures, tokio sync features) appended, leaving the generated bindings
/// referencing crates that aren't in `[dependencies]`. Files with
/// `generated_header: false` are seeds (py.typed markers, sample test files,
/// README.md placeholders) and stay create-only so user edits survive.
pub fn write_scaffold_files_with_overwrite(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<usize> {
    Ok(write_scaffold_files_report(files, base_dir, overwrite)?.changed_count())
}

/// Like [`write_scaffold_files`] but returns the full [`super::write::WriteReport`]
/// (expected vs. actually-changed paths) instead of a bare count.
///
/// NEVER-STAMP / NEVER-OVERWRITE guard: for every file this run is about to write, if a
/// file already exists on disk at that path *and does not already carry an alef marker*,
/// alef has no durable evidence it has ever owned that path — the file is left completely
/// untouched (no header stamp, no content change, `overwrite` notwithstanding), and a
/// warning is logged instead. This is deliberately evaluated *before* any content is
/// prepared (including header stamping), so a foreign file is never even considered for a
/// header it never asked for.
///
/// The check is marker-based, not cache-based, on purpose: `.alef/`'s per-run manifests
/// are gitignored local scratch space (see `cache::read_stage_paths`) that does not survive
/// a fresh clone or a cache-less CI job, so they cannot serve as durable proof of ownership
/// across sessions. A `generated_header: true` file that alef legitimately authored always
/// carries the `alef:hash:` marker in the *committed* file itself from the run that first
/// wrote it onward, which is what makes the marker durable evidence and the cache not.
///
/// This is the fix for the crawlberg incident where a plain `alef all --clean` run (no
/// special flags, no prior claim on either path) silently claimed and stamped hand-written
/// `e2e/go/helpers_test.go` / `e2e/go/main_test.go` with an `alef:hash:` header they never
/// had. `generated_header: false` emits (seeds such as `composer.json`/`package.json`, and
/// formerly Elixir's `test_helper.exs` — see the `~keep` note on its emit site) never carry
/// a marker even when alef legitimately re-authors them every run, so this guard cannot by
/// itself protect *their* hand-edits; the corrected fix for a file in that position is to
/// change its emit to `generated_header: true`, moving it onto the durable rail this guard
/// checks, not to widen this function with cache-derived provenance (see the report for why
/// that path was tried and rejected).
///
/// `poly.toml` is exempt from this guard because [`merge_managed_toml`] intentionally merges
/// project lint configuration. Alef-owned generated TOML manifests are not merge targets: a
/// marker authorizes exact regeneration, while an unmarked manifest is refused. Binary (`.jar`)
/// targets are also exempt because their content cannot be checked for a text marker. ~keep
pub fn write_scaffold_files_report(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<super::write::WriteReport> {
    let mut report = super::write::WriteReport::default();
    let mut prepared = std::collections::BTreeMap::new();
    for file in files {
        if let Some(existing) = prepared.insert(file.path.clone(), file) {
            anyhow::ensure!(
                existing.content == file.content && existing.generated_header == file.generated_header,
                "multiple generators emitted different content for {}",
                file.path.display()
            );
        }
    }
    for file in prepared.into_values() {
        let full_path = base_dir.join(&file.path);
        let can_skip = !overwrite && !file.generated_header && full_path.exists();
        if can_skip {
            report.expected_paths.insert(full_path.clone());
            debug!("  skipped (already exists): {}", full_path.display());
            continue;
        }
        let is_jar_file = full_path.extension().is_some_and(|ext| ext == "jar");
        let is_poly_merge_target = file.path == Path::new(POLY_CONFIG) && full_path.exists();
        // Scoped to `generated_header: true` only: those files are always overwritten
        // regardless of `overwrite` (see the doc comment above), so they are the only
        // ones for which "no marker yet" can mean "never alef's" rather than "an
        // intentionally create-once seed under an explicit --clean". A `generated_header:
        // false` seed (composer.json, package.json, ...) force-written under
        // `overwrite: true` keeps its existing, tested, documented behaviour unchanged.
        // Further scoped to paths alef can actually stamp: an unstampable extension
        // (`.md` above all -- no generated README has ever carried a marker) never gets
        // one even when alef authored every byte, so there a missing marker is not
        // evidence of foreign content, and enforcing on it would freeze regeneration
        // permanently. ~keep
        let is_markable = super::write::marker_comment_style(&full_path).is_some();
        if file.generated_header && is_markable && !is_jar_file && !is_poly_merge_target && full_path.exists() {
            let existing = std::fs::read_to_string(&full_path)
                .with_context(|| format!("failed to read existing {}", full_path.display()))?;
            if !crate::core::hash::content_has_alef_marker(&existing) {
                warn!(
                    "refusing to write {}: pre-existing file carries no alef marker and \
                     alef has no durable record of ever owning it -- leaving it untouched",
                    full_path.display()
                );
                continue;
            }
        }
        report.expected_paths.insert(full_path.clone());
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        if is_jar_file {
            let binary_content = base64::engine::general_purpose::STANDARD
                .decode(&file.content)
                .with_context(|| format!("failed to decode base64 for {}", full_path.display()))?;
            if let Ok(existing) = std::fs::read(&full_path)
                && existing == binary_content
            {
                debug!("  unchanged: {}", full_path.display());
                continue;
            }
            super::write::atomic_write(&full_path, &binary_content)?;
            report.changed_paths.insert(full_path.clone());
            debug!("  wrote (binary): {}", full_path.display());
            continue;
        }
        let content = if is_poly_merge_target {
            let existing = std::fs::read_to_string(&full_path)
                .with_context(|| format!("failed to read existing {}", full_path.display()))?;
            merge_managed_toml(&existing, &file.content)
                .with_context(|| format!("failed to merge existing {}", full_path.display()))?
        } else {
            file.content.clone()
        };
        let normalized = normalize_content(&full_path, &content);
        let normalized = if file.generated_header {
            super::write::ensure_generated_header(&full_path, &normalized)
        } else {
            normalized
        };
        if let Ok(existing) = std::fs::read_to_string(&full_path) {
            let existing_body = crate::core::hash::strip_hash_line(&existing);
            let normalized_body = crate::core::hash::strip_hash_line(&normalized);
            if existing_body == normalized_body {
                apply_shebang_chmod(&full_path, &normalized)?;
                debug!("  unchanged: {}", full_path.display());
                continue;
            }
        }
        super::write::atomic_write(&full_path, normalized.as_bytes())?;
        apply_shebang_chmod(&full_path, &normalized)?;
        report.changed_paths.insert(full_path.clone());
        debug!("  wrote: {}", full_path.display());
        if file.path == Path::new(POLY_CONFIG) {
            normalize_poly_config(&full_path, base_dir);
        }
    }
    Ok(report)
}

/// Repo-root poly config, emitted by the scaffold pass.
const POLY_CONFIG: &str = "poly.toml";

pub(super) fn merge_managed_toml(existing: &str, generated: &str) -> anyhow::Result<String> {
    let mut existing_doc = existing.parse::<toml_edit::DocumentMut>()?;
    let generated_doc = generated.parse::<toml_edit::DocumentMut>()?;
    merge_tables(existing_doc.as_table_mut(), generated_doc.as_table());
    Ok(existing_doc.to_string())
}

fn merge_tables(existing: &mut toml_edit::Table, generated: &toml_edit::Table) {
    for (key, generated_item) in generated {
        match existing.get_mut(key) {
            Some(existing_item) => merge_items(existing_item, generated_item),
            None => {
                existing.insert(key, detached_item(generated_item.clone()));
            }
        }
    }
}

fn merge_items(existing: &mut toml_edit::Item, generated: &toml_edit::Item) {
    match (existing, generated) {
        (toml_edit::Item::Table(existing), toml_edit::Item::Table(generated)) => {
            merge_tables(existing, generated);
        }
        (toml_edit::Item::Value(existing), toml_edit::Item::Value(generated)) => {
            merge_values(existing, generated);
        }
        (existing, generated) => *existing = detached_item(generated.clone()),
    }
}

fn merge_values(existing: &mut toml_edit::Value, generated: &toml_edit::Value) {
    match (existing, generated) {
        (toml_edit::Value::Array(existing), toml_edit::Value::Array(generated)) => {
            for value in generated.iter() {
                let generated_value = value.to_string();
                if !existing
                    .iter()
                    .any(|candidate| candidate.to_string().trim() == generated_value.trim())
                {
                    existing.push(value.clone());
                }
            }
        }
        (toml_edit::Value::InlineTable(existing), toml_edit::Value::InlineTable(generated)) => {
            for (key, generated_value) in generated.iter() {
                match existing.get_mut(key) {
                    Some(existing_value) => merge_values(existing_value, generated_value),
                    None => {
                        existing.insert(key, generated_value.clone());
                    }
                }
            }
        }
        (existing, generated) => {
            let decor = existing.decor().clone();
            *existing = generated.clone();
            *existing.decor_mut() = decor;
        }
    }
}

fn detached_item(mut item: toml_edit::Item) -> toml_edit::Item {
    match &mut item {
        toml_edit::Item::Value(value) => value.decor_mut().clear(),
        toml_edit::Item::Table(table) => {
            table.set_position(None);
            let keys = table.iter().map(|(key, _)| key.to_string()).collect::<Vec<_>>();
            for key in keys {
                if let Some(child) = table.remove(&key) {
                    table.insert(&key, detached_item(child));
                }
            }
        }
        toml_edit::Item::ArrayOfTables(tables) => {
            for table in tables.iter_mut() {
                table.set_position(None);
            }
        }
        toml_edit::Item::None => {}
    }
    item
}

/// Hand `poly.toml` to poly immediately after writing it.
///
/// poly defines the canonical TOML form and the scaffold emits the file from a
/// hand-rolled string template that does not match it, so the file is rewritten on
/// every run and, left raw, fails the consumer's own `poly fmt --check`. The
/// full-regen convergence pass normally repairs it, but that runs many fallible
/// stages later (post-build, stubs, readme, e2e, docs) — an abort in any of them
/// leaves the raw file behind — and the partial-regen paths never pass the repo
/// root to poly at all. Formatting it here closes both gaps for the cost of one
/// single-file invocation. Best-effort: `poly_format` warns and returns when poly
/// is not on PATH.
fn normalize_poly_config(full_path: &Path, base_dir: &Path) {
    crate::cli::pipeline::poly_format(std::slice::from_ref(&full_path.to_path_buf()), base_dir);
}
