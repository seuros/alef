//! Fixture-driven e2e test generation for alef.
//!
//! This crate generates complete, runnable e2e test projects for all supported
//! languages from JSON fixture files. Each project is self-contained with
//! build files, test files, and local package references.

pub mod codegen;
pub mod config;
mod coverage_cache;
pub mod escape;
pub mod field_access;
pub mod fixture;
pub mod format;
pub mod scaffold;
pub mod snippets;
pub mod template_env;
pub mod validate;

use crate::core::backend::GeneratedFile;
use crate::core::config::e2e::DependencyMode;
use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::{Context, Result, bail};
use config::E2eConfig;
use fixture::{group_fixtures, load_fixtures};
use std::path::Path;
use tracing::{info, warn};
use validate::Severity;

/// Map the top-level `[languages]` list (the scaffolded bindings) to the
/// e2e generator names registered in [`codegen::all_generators`].
///
/// `Language::Ffi` maps to the `c` generator (the FFI binding's e2e harness
/// is the C test runner). `Language::Rust` is always appended because rust is
/// the source language and the rust e2e suite exercises the core crate.
///
/// Generators that don't have a corresponding `Language` variant (e.g.
/// `brew`) are intentionally excluded — they require an explicit opt-in via
/// `[e2e].languages` in alef.toml.
pub fn default_e2e_languages(scaffolded: &[Language]) -> Vec<String> {
    let mut names: Vec<String> = scaffolded
        .iter()
        .map(|l| match l {
            Language::Ffi => "c".to_string(),
            other => other.to_string(),
        })
        .collect();
    if !names.iter().any(|n| n == "rust") {
        names.push("rust".to_string());
    }
    names
}

/// ~keep The complete set of names `default_e2e_languages` can ever produce, across
/// every possible `Language` variant — not just the ones scaffolded in the
/// current run.
///
/// Derived by running [`Language::ALL`] through the same mapping
/// `default_e2e_languages` uses, so the two can never drift apart. This is
/// deliberately NOT `core::config::extras::is_known_language`: that function
/// matches on `Language`'s `Display` output and so accepts `"ffi"`, but
/// `default_e2e_languages` maps `Language::Ffi` to the `"c"` generator —
/// `"ffi"` never appears as an actual e2e target name and must stay rejected.
pub fn known_e2e_target_names() -> Vec<String> {
    let mut names = default_e2e_languages(&Language::ALL);
    let mut deduped: Vec<String> = Vec::with_capacity(names.len());
    for name in names.drain(..) {
        if !deduped.contains(&name) {
            deduped.push(name);
        }
    }
    deduped
}

/// Generate e2e test projects from fixtures.
///
/// Returns the list of generated files. The caller is responsible for writing
/// them to disk.
///
/// `type_defs` is the IR type registry for the source crate. Pass
/// `&api.types` from the extracted [`crate::core::ir::ApiSurface`]. It is
/// forwarded to generators that need to introspect struct field types (e.g.
/// the TypeScript/WASM backend uses it to auto-derive `nested_types` for
/// wasm-bindgen class wrapping). Pass an empty slice when the registry is not
/// available; generators will fall back to explicit call-override mappings.
///
/// `enums` is the IR enum registry for the source crate. Pass `&api.enums`
/// from the extracted [`crate::core::ir::ApiSurface`]. For WASM, it is used
/// to identify tagged-data enums so they are emitted as plain JS object literals
/// instead of wrapper factories. Pass an empty slice when not available.
pub fn generate_e2e(
    config: &ResolvedCrateConfig,
    e2e_config: &E2eConfig,
    languages: Option<&[String]>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<Vec<GeneratedFile>> {
    let fixtures_dir = Path::new(&e2e_config.fixtures);
    let fixtures = load_fixtures(fixtures_dir)
        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;

    info!("Loaded {} fixture(s) from {}", fixtures.len(), e2e_config.fixtures);

    // Validate `skip.languages` ids against the full configured target list
    // (not the possibly `--lang`-filtered set below) so the check is stable
    // regardless of which subset of languages this particular invocation
    // generates for.
    let configured_languages: Vec<String> = if !e2e_config.languages.is_empty() {
        e2e_config.languages.clone()
    } else {
        default_e2e_languages(&config.languages)
    };
    fixture::validate_skip_languages(&fixtures, &configured_languages)?;

    // Resolution order for which language generators to run:
    //   1. Explicit `--lang` filter from the CLI (highest priority).
    //   2. `[e2e].languages` from alef.toml when set.
    //   3. The top-level `[languages]` list mapped to e2e generator names —
    //      so e2e tests are only generated for actually scaffolded bindings,
    //      never for backends the consumer hasn't opted into.
    //
    // The legacy `all_generators()` fallback is removed; emitting tests for
    // languages without a matching binding produces broken e2e dirs that
    // cannot compile.
    let resolved_languages: Vec<String> = if let Some(langs) = languages {
        langs.to_vec()
    } else if !e2e_config.languages.is_empty() {
        e2e_config.languages.clone()
    } else {
        default_e2e_languages(&config.languages)
    };

    // Run semantic validation against the resolved language set so the
    // empty-category check warns about the same languages we're about to
    // generate for.
    let diagnostics = validate::validate_fixtures_semantic(&fixtures, e2e_config, &resolved_languages);
    for diag in &diagnostics {
        match diag.severity {
            Severity::Error => warn!("{}: {}", diag.file, diag.message),
            Severity::Warning => warn!("{}: {}", diag.file, diag.message),
        }
    }
    let assertion_recipe_errors: Vec<_> = diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Error && diag.message.contains("requires assertion recipe"))
        .collect();
    if !assertion_recipe_errors.is_empty() {
        bail!(
            "e2e fixture assertion recipe validation failed: {}",
            assertion_recipe_errors
                .iter()
                .map(|diag| format!("{}: {}", diag.file, diag.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }

    let all_groups = group_fixtures(&fixtures);

    // Drop categories that are explicitly excluded from cross-language e2e
    // codegen. These fixtures stay on disk for Rust integration tests but
    // never reach binding generators.
    let all_groups: Vec<_> = if e2e_config.exclude_categories.is_empty() {
        all_groups
    } else {
        all_groups
            .into_iter()
            .filter(|g| !e2e_config.exclude_categories.contains(&g.category))
            .collect()
    };

    // In registry mode with a non-empty category filter, keep only the listed
    // categories so the generated test apps contain a curated subset.
    let groups: Vec<_> =
        if e2e_config.dep_mode == DependencyMode::Registry && !e2e_config.registry.categories.is_empty() {
            let allowed = &e2e_config.registry.categories;
            all_groups
                .into_iter()
                .filter(|g| allowed.iter().any(|c| c == &g.category))
                .collect()
        } else {
            all_groups
        };

    let generators = codegen::generators_for(&resolved_languages);

    let mut all_files = Vec::new();
    for generator in &generators {
        let files = generator.generate(&groups, e2e_config, config, type_defs, enums)?;
        info!("  [{}] generated {} file(s)", generator.language_name(), files.len());
        all_files.extend(files);
    }

    // Let registered extensions contribute e2e files per language. The default
    // `Extension::emit_e2e` returns empty, so consumers without an e2e extension
    // see no change. Returned files merge into the same collection the caller
    // writes and orphan-sweeps.
    crate::with_extensions(|exts| {
        for lang in &resolved_languages {
            for ext in exts {
                let extra = ext.emit_e2e(&groups, e2e_config, config, lang, type_defs, enums)?;
                if !extra.is_empty() {
                    info!(
                        "  [{}] extension `{}` generated {} e2e file(s)",
                        lang,
                        ext.name(),
                        extra.len()
                    );
                }
                all_files.extend(extra);
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;

    if let Some(snippet_config) = &e2e_config.snippets {
        let report = snippets::generate_snippet_report(
            &fixtures,
            snippet_config.languages_or(&resolved_languages),
            e2e_config,
            snippet_config,
            config,
            type_defs,
            enums,
            functions,
        )?;
        report_snippet_coverage(&report.coverage);
        prune_orphaned_snippets(Path::new(&snippet_config.output), &report.coverage);
        ensure_snippet_coverage_complete(&report.coverage)?;
        let coverage_content =
            serde_json::to_string_pretty(&report.coverage).context("failed to serialize snippet coverage manifest")?;
        all_files.push(GeneratedFile {
            path: Path::new(&snippet_config.output).join(snippets::COVERAGE_MANIFEST),
            content: format!("{coverage_content}\n"),
            generated_header: false,
        });
        all_files.extend(report.snippets.into_iter().map(|snippet| snippet.file));
    }

    Ok(all_files)
}

pub fn report_cached_snippet_coverage(path: &Path) -> Result<()> {
    let coverage = coverage_cache::read_coverage_manifest(path)?;
    snippets::coverage::validate(&coverage)?;
    report_snippet_coverage(&coverage);
    ensure_snippet_coverage_complete(&coverage)
}

pub fn evaluate_snippet_coverage(
    config: &ResolvedCrateConfig,
    e2e_config: &E2eConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<Option<snippets::SnippetCoverageLedger>> {
    let Some(snippet_config) = &e2e_config.snippets else {
        return Ok(None);
    };
    let fixtures_dir = Path::new(&e2e_config.fixtures);
    let fixtures = load_fixtures(fixtures_dir)
        .with_context(|| format!("failed to load fixtures from {}", fixtures_dir.display()))?;
    let configured_languages = if e2e_config.languages.is_empty() {
        default_e2e_languages(&config.languages)
    } else {
        e2e_config.languages.clone()
    };
    let report = snippets::generate_snippet_report(
        &fixtures,
        snippet_config.languages_or(&configured_languages),
        e2e_config,
        snippet_config,
        config,
        type_defs,
        enums,
        functions,
    )?;
    Ok(Some(report.coverage))
}

pub fn ensure_fresh_snippet_coverage_complete(coverage: &snippets::SnippetCoverageLedger) -> Result<()> {
    snippets::coverage::validate(coverage)?;
    report_snippet_coverage(coverage);
    ensure_snippet_coverage_complete(coverage)
}

pub fn verify_fresh_snippet_coverage(
    base_dir: &Path,
    config: &ResolvedCrateConfig,
    e2e_config: &E2eConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<()> {
    let Some(snippet_config) = &e2e_config.snippets else {
        return Ok(());
    };
    let computed = evaluate_snippet_coverage(config, e2e_config, type_defs, enums, functions)?
        .expect("snippet configuration produces a coverage ledger");
    ensure_fresh_snippet_coverage_complete(&computed)?;
    let manifest = base_dir.join(&snippet_config.output).join(snippets::COVERAGE_MANIFEST);
    let disk = coverage_cache::read_coverage_manifest(&manifest)?;
    snippets::coverage::validate_tracked_files(&disk, &base_dir.join(&snippet_config.output))?;
    snippets::coverage::validate_current(disk, computed)
        .with_context(|| format!("snippet coverage manifest is stale: {}", manifest.display()))?;
    Ok(())
}

fn report_snippet_coverage(coverage: &snippets::SnippetCoverageLedger) {
    for missing in &coverage.missing {
        warn!(
            "snippet coverage missing for fixture `{}` language `{}`: {}",
            missing.key.fixture_id, missing.key.language, missing.reason
        );
    }
}

fn ensure_snippet_coverage_complete(coverage: &snippets::SnippetCoverageLedger) -> Result<()> {
    let Some(first) = coverage.missing.first() else {
        return Ok(());
    };
    bail!(
        "snippet generation has {} undocumented coverage gap(s); first missing recipe is fixture `{}` language `{}`: {}",
        coverage.missing.len(),
        first.key.fixture_id,
        first.key.language,
        first.reason
    )
}

/// Delete previously alef-generated snippet files that this run no longer
/// produces.
///
/// Without this pass, a fixture that stops rendering (e.g. its recipe starts
/// requiring an extension that isn't registered) keeps its stale
/// previous-run `.md` file on disk forever: [`generate_e2e`] only returns
/// files for keys it *did* generate this run, and the generic
/// `alef:hash:`-header orphan sweeps
/// (`crate::cli::pipeline::sweep_orphans`, `crate::cli::pipeline::sweep_manifest_orphans`)
/// never see it: snippet files are written with `generated_header: false` so
/// they never carry that marker.
///
/// See [`snippets::coverage::orphaned_paths`] for the ownership predicate —
/// only a path recorded in the *previous* run's own coverage manifest is
/// ever a deletion candidate, and only for a language this run actually
/// evaluated. That keeps a `--lang`-filtered or cached run from
/// mass-deleting another language's still-valid output, and keeps a
/// hand-authored file untouched.
///
/// Best-effort: a missing or unreadable previous manifest (first run, or one
/// predating this ledger format) means "nothing to prune yet", not an
/// error — pruning must never turn a routine `generate` into a hard failure.
fn prune_orphaned_snippets(output_root: &Path, coverage: &snippets::SnippetCoverageLedger) {
    let manifest_path = output_root.join(snippets::COVERAGE_MANIFEST);
    let Ok(previous) = coverage_cache::read_coverage_manifest(&manifest_path) else {
        return;
    };
    for relative in snippets::coverage::orphaned_paths(&previous, coverage) {
        let path = output_root.join(&relative);
        match std::fs::remove_file(&path) {
            Ok(()) => info!("Pruned orphaned snippet: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => warn!("failed to prune orphaned snippet {}: {error}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undocumented_missing_recipe_fails_generation() {
        let coverage = snippets::SnippetCoverageLedger {
            missing: vec![snippets::MissingSnippet {
                key: snippets::SnippetCoverageKey {
                    fixture_id: "create_record".into(),
                    language: "go".into(),
                },
                reason: "built-in `go` snippet recipe has no function identity".into(),
            }],
            ..Default::default()
        };

        let error = ensure_snippet_coverage_complete(&coverage).expect_err("missing recipe must fail closed");
        assert!(error.to_string().contains("create_record"));
        assert!(error.to_string().contains("go"));
    }

    #[test]
    fn documented_exceptions_do_not_fail_generation() {
        let coverage = snippets::SnippetCoverageLedger {
            documented_exceptions: vec![snippets::DocumentedSnippetException {
                key: snippets::SnippetCoverageKey {
                    fixture_id: "stream_records".into(),
                    language: "swift".into(),
                },
                reason: "streaming recipe is documented separately".into(),
                reference: "docs/streaming.md".into(),
            }],
            ..Default::default()
        };

        ensure_snippet_coverage_complete(&coverage).expect("documented exception is intentional");
    }

    fn key(fixture_id: &str, language: &str) -> snippets::SnippetCoverageKey {
        snippets::SnippetCoverageKey {
            fixture_id: fixture_id.into(),
            language: language.into(),
        }
    }

    fn metadata(fixture_id: &str, language: &str, path: &str) -> snippets::GeneratedSnippetMetadata {
        snippets::GeneratedSnippetMetadata {
            key: key(fixture_id, language),
            path: std::path::PathBuf::from(path),
            language: language.into(),
            target: language.into(),
            session: language.into(),
            requires: Vec::new(),
            side_effect: crate::e2e::fixture::SideEffectClass::Safe,
        }
    }

    fn write_previous_manifest(output_root: &Path, metadata_entries: Vec<snippets::GeneratedSnippetMetadata>) {
        let ledger = snippets::SnippetCoverageLedger {
            format_version: snippets::COVERAGE_MANIFEST_VERSION,
            generated_paths: metadata_entries.iter().map(|entry| entry.path.clone()).collect(),
            generated: metadata_entries.iter().map(|entry| entry.key.clone()).collect(),
            expected: metadata_entries.iter().map(|entry| entry.key.clone()).collect(),
            generated_metadata: metadata_entries,
            missing: Vec::new(),
            documented_exceptions: Vec::new(),
        };
        std::fs::write(
            output_root.join(snippets::COVERAGE_MANIFEST),
            serde_json::to_string_pretty(&ledger).expect("serialize previous coverage ledger"),
        )
        .expect("write previous coverage manifest");
    }

    /// A fixture that stopped rendering between runs must have its stale
    /// on-disk `.md` file deleted, not left behind for `alef verify` to keep
    /// validating forever. This is task #542.
    #[test]
    fn prune_orphaned_snippets_deletes_a_file_this_run_no_longer_generates() {
        let directory = tempfile::tempdir().expect("temporary output directory");
        let output_root = directory.path();
        let generated_dir = output_root.join("python");
        std::fs::create_dir_all(&generated_dir).expect("python output dir");
        let stale_file = generated_dir.join("register_ocr_backend_trait_bridge.md");
        std::fs::write(&stale_file, "stale 0.60.0 content\n").expect("write stale snippet");

        write_previous_manifest(
            output_root,
            vec![metadata(
                "register_ocr_backend_trait_bridge",
                "python",
                "python/register_ocr_backend_trait_bridge.md",
            )],
        );

        // The key was evaluated this run and rejected: it is in `expected`
        // but produced no file.
        let current = snippets::SnippetCoverageLedger {
            format_version: snippets::COVERAGE_MANIFEST_VERSION,
            expected: vec![key("register_ocr_backend_trait_bridge", "python")],
            missing: vec![snippets::MissingSnippet {
                key: key("register_ocr_backend_trait_bridge", "python"),
                reason: "test-backend fixture requires an extension-owned documentation recipe".into(),
            }],
            ..Default::default()
        };

        prune_orphaned_snippets(output_root, &current);

        assert!(
            !stale_file.exists(),
            "expected {} to be pruned, but it still exists",
            stale_file.display()
        );
    }

    /// A hand-authored `.md` file that alef never generated (absent from the
    /// previous run's `generated_metadata`) must never be deleted, even if a
    /// fixture with a colliding id/language key is reported as `missing`.
    #[test]
    fn prune_orphaned_snippets_never_deletes_a_file_alef_does_not_own() {
        let directory = tempfile::tempdir().expect("temporary output directory");
        let output_root = directory.path();
        let hand_authored_dir = output_root.join("python");
        std::fs::create_dir_all(&hand_authored_dir).expect("python output dir");
        let hand_authored_file = hand_authored_dir.join("register_ocr_backend_trait_bridge.md");
        std::fs::write(&hand_authored_file, "hand-authored recipe, not alef output\n").expect("write hand-authored");

        // The previous manifest never claims this path: alef never generated it.
        write_previous_manifest(output_root, Vec::new());

        let current = snippets::SnippetCoverageLedger {
            format_version: snippets::COVERAGE_MANIFEST_VERSION,
            expected: vec![key("register_ocr_backend_trait_bridge", "python")],
            missing: vec![snippets::MissingSnippet {
                key: key("register_ocr_backend_trait_bridge", "python"),
                reason: "test-backend fixture requires an extension-owned documentation recipe".into(),
            }],
            ..Default::default()
        };

        prune_orphaned_snippets(output_root, &current);

        assert!(
            hand_authored_file.exists(),
            "hand-authored file must survive: {}",
            hand_authored_file.display()
        );
    }

    #[test]
    fn generation_does_not_write_fixture_schema() {
        let directory = tempfile::tempdir().expect("temporary fixture directory");
        let e2e_config = E2eConfig {
            fixtures: directory.path().display().to_string(),
            ..E2eConfig::default()
        };

        generate_e2e(&ResolvedCrateConfig::default(), &e2e_config, Some(&[]), &[], &[], &[])
            .expect("generate empty E2E suite");

        assert!(!directory.path().join("schema.json").exists());
    }
}
