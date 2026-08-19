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
pub mod validate_call_class;

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

/// The complete set of e2e generator target names alef knows about: every
/// [`codegen::E2eCodegen`] registered in [`codegen::all_generators`], not just the
/// subset reachable from the top-level `[languages]` scaffold list.
///
/// ~keep This must read the generator registry directly rather than running
/// [`Language::ALL`] through [`default_e2e_languages`] (an earlier version of this
/// function did exactly that). That derivation is blind to generators with no
/// corresponding `Language` variant -- `brew`, `homebrew`, `php_ext` are legitimate,
/// opt-in-only e2e targets (see `default_e2e_languages`'s doc) that a fixture author
/// can still validly hold a `skip.languages` entry for, and it let `"jni"` through as
/// a "known" target even though no e2e generator is registered under that name, so a
/// skip declared against it could never match anything. Sourcing this list from
/// [`codegen::all_generators`] instead means it can never diverge from the generators
/// that actually run, and it is the same source `snippets::known_language_names`
/// validates `docs.coverage_exceptions` keys against, so the two "is this a real
/// backend" checks in the e2e pipeline can't drift apart from each other either.
pub fn known_e2e_target_names() -> Vec<String> {
    let mut names: Vec<String> = codegen::all_generators()
        .iter()
        .map(|generator| generator.language_name().to_string())
        .collect();
    names.sort();
    names.dedup();
    names
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
///
/// `functions` is the IR free-function registry. Pass `&api.functions`. It reaches
/// both the generated-test-file path ([`codegen::E2eCodegen::generate`]) and the
/// documentation-snippet path, so a call's result type resolves from the declared
/// return type rather than from a PascalCased guess at the call name. Pass an empty
/// slice when not available; generators fall back to the guess.
/// Thin wrapper: reads the active extensions from the process-global registry and
/// delegates to [`generate_e2e_with_extensions`]. Every production caller keeps calling
/// this function unchanged -- the split below exists solely so tests can inject a
/// synthetic extensions list instead of mutating `crate::EXTENSIONS`, which is a
/// process-global `OnceLock` settable exactly once per test binary and therefore unsafe
/// to touch from any individual test (see `generate_e2e_with_extensions`'s own tests).
/// Mirrors `snippets::generate_snippet_report` / `generate_snippet_report_with_extensions`
/// exactly, for the same reason. ~keep
pub fn generate_e2e(
    config: &ResolvedCrateConfig,
    e2e_config: &E2eConfig,
    languages: Option<&[String]>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) -> Result<(Vec<GeneratedFile>, Option<anyhow::Error>)> {
    crate::with_extensions(|extensions| {
        generate_e2e_with_extensions(
            config, e2e_config, languages, type_defs, enums, functions, errors, extensions,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn generate_e2e_with_extensions(
    config: &ResolvedCrateConfig,
    e2e_config: &E2eConfig,
    languages: Option<&[String]>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
    extensions: &[Box<dyn crate::Extension>],
) -> Result<(Vec<GeneratedFile>, Option<anyhow::Error>)> {
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

    // Field-classification validation runs BEFORE any generator, because the failure it replaces
    // is a compiler diagnostic pointed at generated code: a `fields_optional` entry naming a
    // field the IR declares non-optional emits an Option unwrap against a concrete value, and the
    // operator sees "type annotations needed" in a file they never wrote, with nothing naming the
    // config line that caused it. Refusing here costs one regeneration; letting it through costs
    // a debugging session in the wrong tree. ~keep
    let classification_diagnostics = validate::validate_field_classifications(e2e_config, type_defs);
    for diag in &classification_diagnostics {
        warn!("{}: {}", diag.file, diag.message);
    }
    let classification_errors: Vec<_> = classification_diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Error)
        .collect();
    if !classification_errors.is_empty() {
        bail!(
            "e2e field-classification validation failed: {}",
            classification_errors
                .iter()
                .map(|diag| format!("{}: {}", diag.file, diag.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }

    // `class` override validation runs for the same reason: an unresolvable value is
    // trusted blindly by every consuming generator today, and the mismatch used to
    // surface only as a wall of "cannot find symbol" compile errors in generated code
    // far downstream, with nothing pointing back at the config line that caused it.
    let class_override_diagnostics =
        validate_call_class::validate_call_class_overrides(e2e_config, config, type_defs, enums, &resolved_languages);
    for diag in &class_override_diagnostics {
        warn!("{}: {}", diag.file, diag.message);
    }
    let class_override_errors: Vec<_> = class_override_diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Error)
        .collect();
    if !class_override_errors.is_empty() {
        bail!(
            "e2e call class override validation failed: {}",
            class_override_errors
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

    let (mut all_files, generator_failures) = run_generators(
        &generators,
        &groups,
        e2e_config,
        config,
        type_defs,
        enums,
        functions,
        errors,
    );

    // A backend's codegen failure no longer turns this function into an `Err`: it travels
    // out alongside `all_files` in this slot instead, so the caller can still write every
    // file that did succeed -- including the failing backend's own siblings -- and stamp
    // their provenance before it ever has to decide whether to propagate. Returning `Err`
    // here, as this used to, discarded `all_files` wholesale even though `run_generators`
    // had already isolated the failure to just the one backend; deferring it is what makes
    // that isolation reach the caller instead of stopping at this function's boundary. The
    // extension-emission and snippet-stage blocks below can also populate this slot -- see
    // the comments there -- and `Option::get_or_insert` keeps whichever failure claims it
    // first rather than losing any of them. ~keep
    let mut deferred_error = ensure_no_generator_failures(&generator_failures, generators.len());

    // Every assertion a backend dropped rather than rendered is on the ledger by now. Report the
    // count unconditionally -- a skip that is legitimate today is still an assertion that is not
    // running, and this line is the only thing that keeps that debt visible -- then defer the
    // strict failure alongside the generator failures above, so the files that did generate are
    // still written and the consumer sees every unresolved field path in one error. ~keep
    let skip_records = codegen::take_skip_records();
    if let Some(summary) = codegen::skip_summary(&skip_records) {
        info!("{summary}");
    }
    if let Some(error) = codegen::strict_assertion_failure(&skip_records, codegen::strict_assertions_enabled()) {
        warn!("e2e strict-assertion check failed, deferring failure");
        deferred_error.get_or_insert(error);
    }

    // ~keep An example every one of whose assertions funnelled into a skip marker is as inert as
    // one with no assertions at all, and the skip summary above cannot say so: it counts markers,
    // and a body of three markers is indistinguishable there from a body of three markers plus a
    // real check. The backends refuse to publish those as passing tests; this is where the count
    // becomes visible, at WARN because a refused example is coverage that is not running.
    let inert_examples = codegen::inert_example::take_inert_examples();
    if let Some(summary) = codegen::inert_example::inert_summary(&inert_examples) {
        warn!("{summary}");
    }

    // Let registered extensions (passed in by the caller -- see `generate_e2e`'s doc
    // comment) contribute e2e files per language. The default `Extension::emit_e2e`
    // returns empty, so consumers without an e2e extension see no change. Returned files
    // merge into the same collection the caller writes and orphan-sweeps.
    //
    // A failure here defers into `deferred_error` the same way the generator failure
    // above and the snippet-stage failures below do, instead of aborting via `?`: `emit_e2e` is
    // documented as emitting one self-contained file set per `(language, extension)`
    // call with no state carried between calls (it does not even receive `all_files`),
    // so a failing call contributes zero files for that pair -- never a partial one --
    // and leaves every earlier pair's already-merged output exactly as coherent as it
    // already was. There is nothing this call could have half-written that unwinding
    // `all_files` would protect against. The immediately-invoked closure below (rather
    // than a bare `?` in the loop) exists only to give that `?` an early-exit boundary
    // narrower than this whole function, so a failure skips the rest of the extension
    // loop without skipping the snippet stage or the return below. ~keep
    let extension_result = (|| {
        for lang in &resolved_languages {
            for ext in extensions {
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
    })();
    if let Err(error) = extension_result {
        warn!("e2e extension emission failed, deferring failure: {error:#}");
        deferred_error.get_or_insert(error);
    }

    // A snippet-stage failure defers into `deferred_error` the same way a generator
    // failure does above, instead of aborting via `?`: the generators that already ran
    // successfully -- CODE, not documentation -- must not be discarded because the
    // snippet stage behind them tripped. `Option::get_or_insert` leaves an earlier
    // generator failure in place rather than overwriting it with this one. ~keep
    if let Some(snippet_config) = &e2e_config.snippets {
        match snippets::generate_snippet_report(
            &fixtures,
            snippet_config.languages_or(&resolved_languages),
            e2e_config,
            snippet_config,
            config,
            type_defs,
            enums,
            functions,
        ) {
            Ok(report) => {
                report_snippet_coverage(&report.coverage);
                // The prune must not run unless the coverage that licenses it is complete.
                // `orphaned_paths` stakes its whole safety argument on this gate having passed
                // first: a snippet that merely FAILED TO RENDER is absent from `generated_paths`
                // while its language is still in `expected`, which is indistinguishable from a
                // genuine orphan -- so pruning first unlinks previously published documentation
                // because of a transient generator failure. The gate only DEFERS its error, so
                // ordering alone is not enough; the prune has to be skipped explicitly. ~keep
                match ensure_snippet_coverage_complete(&report.coverage) {
                    Ok(()) => prune_orphaned_snippets(Path::new(&snippet_config.output), &report.coverage),
                    Err(error) => {
                        warn!(
                            "snippet coverage incomplete, deferring failure and skipping the orphan prune: {error:#}"
                        );
                        deferred_error.get_or_insert(error);
                    }
                }
                let coverage_content = serde_json::to_string_pretty(&report.coverage)
                    .context("failed to serialize snippet coverage manifest")?;
                all_files.push(GeneratedFile {
                    path: Path::new(&snippet_config.output).join(snippets::COVERAGE_MANIFEST),
                    content: format!("{coverage_content}\n"),
                    generated_header: false,
                });
                all_files.extend(report.snippets.into_iter().map(|snippet| snippet.file));
            }
            Err(error) => {
                warn!("snippet generation failed, deferring failure: {error:#}");
                deferred_error.get_or_insert(error);
            }
        }
    }

    Ok((all_files, deferred_error))
}

/// Run every per-language e2e generator, isolating one backend's codegen failure from
/// every other backend and from the snippet stage that runs after this in
/// [`generate_e2e`].
///
/// Before this, the loop propagated a generator's `Err` with `?` immediately, which
/// made one backend's localized problem abort the entire regen: every later-listed
/// language never ran, and the snippet stage -- gated on this whole function returning
/// `Ok` -- never started either, even though it does not read `all_files` and has
/// nothing to do with the failing backend. That is not hypothetical: a consumer's C
/// backend hit `ensure_leaf_field_exists`'s deliberate `bail!` (see
/// `codegen::c::assertions::ensure_leaf_field_exists`) and the resulting abort left
/// their snippet and docs trees stale for two days with `git status` reading clean,
/// because nothing downstream of the C generator ever ran long enough to write
/// anything. A backend's `bail!` is still the right call *for that backend* -- emitting
/// a call to a symbol that does not exist is worse than skipping it -- but the fix
/// belongs at this level, in how one backend's refusal is allowed to affect its
/// siblings, not by weakening the per-symbol check itself. Each failure is still
/// reported at `WARN` immediately (this repo's level contract: degraded, but the run
/// continues) with the backend's own diagnostic verbatim, and the caller still sees a
/// hard `Err` once every backend that could run has. ~keep
#[allow(clippy::too_many_arguments)]
fn run_generators(
    generators: &[Box<dyn codegen::E2eCodegen>],
    groups: &[fixture::FixtureGroup],
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    errors: &[crate::core::ir::ErrorDef],
) -> (Vec<GeneratedFile>, Vec<String>) {
    let mut all_files = Vec::new();
    let mut failures = Vec::new();
    for generator in generators {
        match generator.generate_gated(groups, e2e_config, config, type_defs, enums, functions, errors) {
            Ok(files) => {
                info!("  [{}] generated {} file(s)", generator.language_name(), files.len());
                all_files.extend(files);
            }
            Err(error) => {
                warn!(
                    "  [{}] e2e codegen failed, skipping this backend: {error:#}",
                    generator.language_name()
                );
                failures.push(format!("[{}] {error:#}", generator.language_name()));
            }
        }
    }
    (all_files, failures)
}

/// Turn the failures [`run_generators`] collected into the single deferred error
/// [`generate_e2e`] carries out to its caller once every backend that could run already
/// has, instead of a `Result` its caller would have to propagate immediately with `?`.
fn ensure_no_generator_failures(failures: &[String], generator_count: usize) -> Option<anyhow::Error> {
    if failures.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(
        "e2e codegen failed for {} of {} backend(s) -- other backends and the snippet stage still ran: {}",
        failures.len(),
        generator_count,
        failures.join("; ")
    ))
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

        let (_files, deferred_error) = generate_e2e(
            &ResolvedCrateConfig::default(),
            &e2e_config,
            Some(&[]),
            &[],
            &[],
            &[],
            &[],
        )
        .expect("generate empty E2E suite");

        assert!(
            deferred_error.is_none(),
            "an empty fixture set has no backend to fail: {deferred_error:?}"
        );
        assert!(!directory.path().join("schema.json").exists());
    }

    /// The real-world trigger behind the `adopt`/`verify` deadlock this crate's release
    /// blocker fixed: a fixture asserting on a field the availability oracle rejects, with
    /// no `skip` declared, must still fail strict mode by default -- proving the condition
    /// `bin_cli::helpers::collect_managed_surface`'s `StageFailure` handling exists to
    /// tolerate is genuinely reachable through the real generation pipeline, not only
    /// through `codegen::strict_assertion_failure`'s own synthetic-string unit tests. ~keep
    #[test]
    fn an_unresolvable_field_with_no_skip_fails_strict_mode_through_the_real_pipeline() {
        let directory = tempfile::tempdir().expect("temporary fixture directory");
        std::fs::write(
            directory.path().join("smoke.json"),
            r#"{
                "id": "smoke",
                "description": "smoke test",
                "assertions": [
                    { "type": "not_empty", "field": "bogus_field_xyz" }
                ]
            }"#,
        )
        .expect("write fixture");

        let e2e_config = E2eConfig {
            fixtures: directory.path().display().to_string(),
            languages: vec!["python".to_string()],
            result_fields: std::collections::HashSet::from(["id".to_string()]),
            call: crate::core::config::e2e::CallConfig {
                function: "complete".to_string(),
                module: "gatelib".to_string(),
                ..crate::core::config::e2e::CallConfig::default()
            },
            ..E2eConfig::default()
        };

        let (_files, deferred_error) =
            generate_e2e(&ResolvedCrateConfig::default(), &e2e_config, None, &[], &[], &[], &[])
                .expect("the files that did render must still be returned alongside a deferred failure");

        let error = deferred_error.expect("an unresolvable field with no `skip` must fail strict mode by default");
        assert!(
            format!("{error:#}").contains("e2e assertion(s) reference a field the availability oracle cannot resolve"),
            "got: {error:#}"
        );
    }

    struct FailingGenerator;

    impl codegen::E2eCodegen for FailingGenerator {
        fn generate(
            &self,
            _groups: &[fixture::FixtureGroup],
            _e2e_config: &E2eConfig,
            _config: &ResolvedCrateConfig,
            _type_defs: &[crate::core::ir::TypeDef],
            _enums: &[crate::core::ir::EnumDef],
            _functions: &[crate::core::ir::FunctionDef],
            _errors: &[crate::core::ir::ErrorDef],
        ) -> Result<Vec<GeneratedFile>> {
            anyhow::bail!("simulated leaf-field resolution failure")
        }

        fn language_name(&self) -> &'static str {
            "failing"
        }
    }

    struct SucceedingGenerator;

    impl codegen::E2eCodegen for SucceedingGenerator {
        fn generate(
            &self,
            _groups: &[fixture::FixtureGroup],
            _e2e_config: &E2eConfig,
            _config: &ResolvedCrateConfig,
            _type_defs: &[crate::core::ir::TypeDef],
            _enums: &[crate::core::ir::EnumDef],
            _functions: &[crate::core::ir::FunctionDef],
            _errors: &[crate::core::ir::ErrorDef],
        ) -> Result<Vec<GeneratedFile>> {
            Ok(vec![GeneratedFile {
                path: std::path::PathBuf::from("ok/output.txt"),
                content: "generated".into(),
                generated_header: false,
            }])
        }

        fn language_name(&self) -> &'static str {
            "succeeding"
        }
    }

    /// The regression this guards: a consumer's C backend hit `ensure_leaf_field_exists`'s
    /// `bail!`, and because the old loop propagated it with `?` immediately, every
    /// later-listed language generator was skipped too -- not just the C backend. One
    /// backend's codegen failure must not stop its siblings from generating.
    #[test]
    fn run_generators_isolates_one_backend_failure_from_the_rest() {
        let generators: Vec<Box<dyn codegen::E2eCodegen>> =
            vec![Box::new(FailingGenerator), Box::new(SucceedingGenerator)];

        let (files, failures) = run_generators(
            &generators,
            &[],
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        );

        assert_eq!(
            files.len(),
            1,
            "the succeeding backend's file must still be produced: {files:?}"
        );
        assert_eq!(files[0].path, std::path::PathBuf::from("ok/output.txt"));
        assert_eq!(
            failures.len(),
            1,
            "the failing backend's failure must be recorded: {failures:?}"
        );
        assert!(
            failures[0].contains("[failing]") && failures[0].contains("simulated leaf-field resolution failure"),
            "failure must name the backend and carry its own diagnostic verbatim: {failures:?}"
        );
    }

    struct FailingExtension;

    impl crate::Extension for FailingExtension {
        fn name(&self) -> &str {
            "failing-e2e-extension"
        }

        fn emit_e2e(
            &self,
            _groups: &[fixture::FixtureGroup],
            _e2e_config: &E2eConfig,
            _config: &ResolvedCrateConfig,
            _language: &str,
            _type_defs: &[crate::core::ir::TypeDef],
            _enums: &[crate::core::ir::EnumDef],
        ) -> Result<Vec<GeneratedFile>> {
            anyhow::bail!("simulated extension emission failure")
        }
    }

    /// Regression for the same defect class as
    /// `run_generators_isolates_one_backend_failure_from_the_rest`, one layer over: an e2e
    /// extension's `emit_e2e` failure must not discard the backend generator output
    /// `run_generators` already merged into `all_files`, and must still surface as a deferred
    /// failure rather than being swallowed.
    ///
    /// Exercised through `generate_e2e_with_extensions` (a synthetic extensions list passed
    /// directly) instead of the public `generate_e2e` (which reads `crate::EXTENSIONS`, a
    /// process-global `OnceLock` settable exactly once per process) -- no individual test can
    /// mutate that global without leaking its registration into every other test sharing this
    /// binary for the rest of the process's lifetime. `generate_e2e_with_extensions` exists so
    /// this failure mode is provable without that. ~keep
    #[test]
    fn generate_e2e_with_extensions_defers_an_extension_failure_so_backend_output_survives() {
        let directory = tempfile::tempdir().expect("temporary fixture directory");
        let e2e_config = E2eConfig {
            fixtures: directory.path().display().to_string(),
            output: "e2e".to_string(),
            languages: vec!["rust".to_string()],
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig::default();
        let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FailingExtension)];

        let (files, deferred_error) =
            generate_e2e_with_extensions(&config, &e2e_config, None, &[], &[], &[], &[], &extensions)
                .expect("an extension failure must defer through the `Option` slot, not propagate as a hard `Err`");

        let error = deferred_error.expect("the extension's failure must still be reported, not silently dropped");
        assert!(
            format!("{error:#}").contains("simulated extension emission failure"),
            "the deferred error must carry the extension's own diagnostic verbatim: {error:#}"
        );

        assert!(
            files
                .iter()
                .any(|file| file.path == *std::path::Path::new("e2e/rust/Cargo.toml")),
            "the `rust` backend's own e2e suite must survive an unrelated extension's failure: {files:?}"
        );
    }

    #[test]
    fn ensure_no_generator_failures_passes_through_when_nothing_failed() {
        assert!(
            ensure_no_generator_failures(&[], 3).is_none(),
            "no failures must not produce a deferred error"
        );
    }

    #[test]
    fn ensure_no_generator_failures_names_every_failed_backend_and_the_total_count() {
        let failures = vec![
            "[c] simulated leaf-field resolution failure".to_string(),
            "[go] simulated template error".to_string(),
        ];

        let message = ensure_no_generator_failures(&failures, 5)
            .expect("collected failures must still produce a deferred error")
            .to_string();

        assert!(
            message.contains("2 of 5"),
            "must report how many of the total backends failed: {message}"
        );
        assert!(
            message.contains("[c] simulated leaf-field resolution failure"),
            "{message}"
        );
        assert!(message.contains("[go] simulated template error"), "{message}");
    }
}
