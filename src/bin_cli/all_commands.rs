use anyhow::{Context as _, Result};
use std::path::PathBuf;

use crate::cli::{cache, dispatch, pipeline, version_pin};

use super::args::*;
use super::dispatch::DispatchContext;
use super::helpers::*;
mod docs_stage;
mod preflight;
mod stage_failures;
use stage_failures::StageFailures;
/// Surface formatting steps that did not run: registry-mode dependency resolution deferred
/// to a post-publish pass, and formatters whose executable is absent.
///
/// Deliberately not an error, and deliberately routed through the same reporter the
/// standalone stage commands use (`e2e::format::warn_deferred`) rather than classifying the
/// entries a second time here -- this copy used to blame every entry, including a missing
/// formatter, on an unpublished version. Registry-mode manifests pin the version the current
/// run produces, so those steps cannot succeed until that version is published; failing here
/// would mean every release run fails on a precondition required to be false at that moment.
/// Local-mode e2e, which is what actually gates correctness, still hard-fails on any
/// formatter error. ~keep
fn report_deferred_formatting(crate_name: &str, deferred: &[crate::e2e::format::DeferredFormatting]) {
    crate::e2e::format::warn_deferred_for_crate(crate_name, deferred);
}

/// Whether `docs.snippets.validation_level` runs snippets against a built
/// language artifact rather than just parsing/checking syntax.
///
/// `alef all` never builds those artifacts: its own doc comment scopes it to
/// "generate + stubs + scaffold + readme + docs + sync + e2e", and the only
/// build it performs is the narrow one in `complete_generated_artifacts`
/// (post-build hooks plus, only for `Language::Ffi`, the native cdylib --
/// see `bin_cli/helpers.rs`). The full per-language build
/// (`pipeline::build`, e.g. `npm run build` / `mvn compile` / `swift build` /
/// `zig build`) only runs from the standalone `alef build` command. A
/// `typecheck`/`compile`/`run` level configured without that build having
/// run separately fails every affected snippet with a toolchain error whose
/// real cause -- the missing artifact -- is not stated anywhere. ~keep
fn snippet_validation_needs_build_artifacts(validation_level: Option<&str>) -> bool {
    matches!(
        validation_level.map(str::to_ascii_lowercase).as_deref(),
        Some("typecheck" | "compile" | "run")
    )
}

/// Surface the unbuilt-artifact precondition above, once per crate, before the
/// docs stage runs snippet validation -- so it is diagnosable at the top of
/// the run instead of inferred from a flood of per-snippet compiler errors
/// further down. Does not skip or gate the docs stage: `alef all` has no way
/// to know here whether a prior `alef build` already satisfied it. ~keep
fn warn_if_snippet_validation_needs_build(config: &crate::core::config::ResolvedCrateConfig) {
    let Some(level) = config
        .docs
        .as_ref()
        .and_then(|docs| docs.snippets.as_ref())
        .and_then(|snippets| snippets.validation_level.as_deref())
    else {
        return;
    };
    if !snippet_validation_needs_build_artifacts(Some(level)) {
        return;
    }
    tracing::warn!(
        "[{}] docs.snippets.validation_level = \"{level}\" checks snippets against built language \
         artifacts, but `alef all` does not build them -- run `alef build` first. If those artifacts \
         are missing or stale, snippet validation fails with per-snippet toolchain errors whose real \
         cause is the missing build, not the snippet.",
        config.name
    );
}

/// Paths this run's ownership guard refused to write that fall inside the crate's
/// configured `docs.snippets` roots (`dirs`, `inline_dirs`, minus `exclude`).
///
/// Mirrors the directory-level inclusion/exclusion `docs::build_snippet_context` applies
/// before `discover_snippets` walks disk -- not a re-implementation of snippet discovery
/// itself, just enough to tell whether a refusal earlier in this run landed inside the
/// tree that `generate_docs_stage`'s snippet validation later reads back off disk. A
/// non-empty result means that validation -- pass or fail -- was graded against bytes
/// this run never wrote, which is invisible unless something correlates the two. ~keep
fn refused_snippet_dir_paths(
    refused_paths: &std::collections::BTreeSet<PathBuf>,
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Vec<PathBuf> {
    let Some(snippet_cfg) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Vec::new();
    };
    let snippet_dirs: Vec<PathBuf> = snippet_cfg
        .dirs
        .iter()
        .chain(&snippet_cfg.inline_dirs)
        .map(|dir| base_dir.join(dir))
        .collect();
    if snippet_dirs.is_empty() {
        return Vec::new();
    }
    let excluded: Vec<PathBuf> = snippet_cfg.exclude.iter().map(|dir| base_dir.join(dir)).collect();
    refused_paths
        .iter()
        .filter(|path| snippet_dirs.iter().any(|dir| path.starts_with(dir)))
        .filter(|path| !excluded.iter().any(|prefix| path.starts_with(prefix)))
        .cloned()
        .collect()
}

fn sync_registry_versions_before_all(
    config_path: &std::path::Path,
    configs: &[&crate::core::config::ResolvedCrateConfig],
) -> Result<bool> {
    let mut versions = std::collections::BTreeSet::new();
    for config in configs {
        let version = config.resolved_version().with_context(|| {
            format!(
                "could not resolve version for crate `{}` from {}",
                config.name, config.version_from
            )
        })?;
        versions.insert(version);
    }
    anyhow::ensure!(
        versions.len() <= 1,
        "alef all cannot synchronize one registry config from multiple crate versions: {}",
        versions.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let Some(version) = versions.into_iter().next() else {
        return Ok(false);
    };
    pipeline::sync_registry_package_versions(config_path, &version)
}

/// The `overwrite` argument `alef all`'s create-once-bearing write stages hand to
/// [`pipeline::write_scaffold_files_report`].
///
/// `clean` is taken and deliberately does not participate. Until this function existed the
/// scaffold and docs stages passed `clean` straight through, which made one flag mean two
/// unrelated things: "ignore cached IR" and "disable the create-only branch that leaves a
/// pre-existing unmarked file alone". Only the second one destroys work — a hand-grown
/// `composer.json` becomes alef's placeholder — and nothing about wanting a cache-cold rerun
/// implies wanting that. The parameter stays in the signature so the separation is an
/// executable fact with a test behind it rather than an absence a later edit can silently
/// undo by reaching for the `clean` that is already in scope at both call sites. ~keep
pub(crate) fn create_once_overwrite(clean: bool, clobber_create_once_seeds: bool) -> bool {
    let _ = clean;
    clobber_create_once_seeds
}

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    match command {
        Commands::All {
            clean,
            clobber_create_once_seeds,
            skip_frb,
            strict,
            skip_snippet_validation,
        } => {
            if skip_frb {
                let existing = std::env::var("ALEF_SKIP_COMMANDS").unwrap_or_default();
                let updated = if existing.is_empty() {
                    "flutter_rust_bridge_codegen".to_string()
                } else {
                    format!("{existing},flutter_rust_bridge_codegen")
                };
                // SAFETY: single-threaded CLI dispatch; no concurrent env access here.
                unsafe { std::env::set_var("ALEF_SKIP_COMMANDS", updated) };
            }
            let _ = skip_frb;
            let overwrite_create_once = create_once_overwrite(clean, clobber_create_once_seeds);
            let (mut workspace, mut resolved) = load_config(config_path)?;
            version_pin::check_alef_toml_version(&workspace)?;
            let registry_versions_changed = {
                let selected = dispatch::select_crates(&resolved, &context.crate_filter)?;
                sync_registry_versions_before_all(config_path, &selected)?
            };
            if registry_versions_changed {
                (workspace, resolved) = load_config(config_path)?;
                version_pin::check_alef_toml_version(&workspace)?;
            }
            let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
            let multi = dispatch::is_multi_crate(&crates_to_process);
            let base_dir = std::env::current_dir()?;

            // Deferred across the whole run, not just this check -- see `StageFailures`'s doc
            // comment. A rejection here must still fail `alef all`, but it must not fail it
            // before the main loop below has had a chance to write bindings, e2e suites and
            // docs for every crate. ~keep
            let mut stage_failures = StageFailures::new();
            preflight::run_snippet_coverage_preflight(&crates_to_process, config_path, &mut stage_failures);

            let config_toml = std::fs::read_to_string(config_path)?;
            let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

            let mut grand_binding_count: usize = 0;
            let mut grand_stub_count: usize = 0;
            let mut grand_api_count: usize = 0;
            let mut grand_scaffold_count: usize = 0;
            let mut grand_readme_count: usize = 0;
            let mut grand_e2e_count: usize = 0;
            let mut grand_doc_count: usize = 0;
            // A refusal is a run-level fact addressed to an operator, so it is accumulated across
            // every writing phase and reported once at the end. Reporting per phase is how this
            // command came to surface only the scaffold phase's refusals while omitting every
            // binding-phase one from the summary entirely. ~keep
            let mut refusals = pipeline::WriteReport::default();
            // A per-crate docs/snippet validation failure must not short-circuit formatting, orphan
            // sweeping, hash finalisation, deferred-formatting reporting or hook installation -- for
            // this crate or for any crate later in this loop -- because the bindings those steps act
            // on are already written to disk by the time the docs stage runs. Returning early there
            // left them unformatted and unstamped, and an unstamped file has no provenance marker for
            // the ownership guard to recognise next run, which manufactures fresh refusals from a
            // failure that had nothing to do with writing. The first failure is what this function's
            // `Result` reports; later ones are only `tracing::error!`-ed so a second crate's distinct
            // docs failure in the same run is never silently dropped. ~keep
            let mut docs_stage_error: Option<anyhow::Error> = None;
            // A generator failure inside either e2e stage below (`crate::e2e::generate_e2e`) must be
            // deferred the same way, and for a sharper reason than the docs case: the two lines right
            // after each write -- `sweep_manifest_orphans` and `cache::write_stage_hash` -- are
            // actively unsafe to run when a backend's codegen failed. `write_stage_hash` would record
            // this IR+config+fixture hash as satisfied, so the *next* run reads it back as cached,
            // never calls `generate_e2e` again, and exits 0 with the failing backend's suite
            // permanently missing. `sweep_manifest_orphans` compares this run's (incomplete) path set
            // against the last good run's (complete) one, so the previously-working backend's own
            // output -- present in the old set, absent from this one -- reads as orphaned and gets
            // deleted. Both call sites below gate on this being `None` before either line runs; write,
            // format and `finalize_hashes` still run unconditionally, so a partially generated suite
            // still ships settled, stamped bytes. (An earlier revision claimed the reason was that
            // "unstamped output has no provenance marker for the ownership guard": false -- provenance
            // is the prose header marker `write_files_report` writes, not the `alef:hash:` line, and
            // that false claim is what justified the per-phase checkpoints removed from this loop.) ~keep
            let mut e2e_stage_error: Option<anyhow::Error> = None;

            for resolved_cfg in &crates_to_process {
                let languages = resolve_languages(resolved_cfg, None)?;
                pipeline::warn_missing_formatters(&languages);
                if multi {
                    tracing::info!(
                        "[{}] Running all for: {}",
                        resolved_cfg.name,
                        format_languages(&languages)
                    );
                } else {
                    tracing::info!("Running all for: {}", format_languages(&languages));
                }

                let api = pipeline::extract(resolved_cfg, config_path, clean)?;
                let sources_hash = cache::sources_hash(&resolved_cfg.sources)?;

                // Accumulated across every phase below and stamped exactly ONCE, by the
                // `finalize_hashes_sweeping` call after the format pass at the end of this loop
                // body. Every phase used to stamp its own output as soon as it was written -- ten
                // `finalize_hashes(&current_gen_paths, ..)` checkpoints, all ahead of the only
                // formatting pass this command runs -- and a stamped file is one poly refuses to
                // format, so those checkpoints made the format pass a no-op for everything `alef
                // all` emitted (21 of 93 files on a measured eight-language fixture). See
                // `pipeline::format::stamp_gate`'s module doc for the mechanism and the measurements.
                // Losing the checkpoints costs the ownership guard nothing: provenance is the prose
                // header marker `write_files_report` writes, not the `alef:hash:` line. ~keep
                let mut current_gen_paths = std::collections::HashSet::new();
                // Whether formatting is needed this run -- covers every write phase (bindings,
                // service API, stubs, public API, scaffold, e2e/test-apps, README, docs), not just
                // bindings/service-API/stubs. A single `HashSet<Language>` populated from only those
                // three phases (`changed_languages`, pre-fix) under-triggered `format_generated`:
                // a run that only rewrote e.g. scaffold or README output left that phase's own
                // `report.changed_count() > 0` unread by the gate, so the whole-tree converging pass
                // never ran and the newly written file stayed unformatted with a stale hash (alef
                // #119). Seeded from `languages_have_post_build_steps` because a post-build step
                // (e.g. Dart's `flutter_rust_bridge_codegen`) runs unconditionally every pass and
                // writes straight to disk with no `WriteReport` at all -- see that function's doc
                // comment for why its mere presence must count as "may have changed". ~keep
                let mut any_output_changed = languages_have_post_build_steps(&languages, resolved_cfg);
                // Registry-mode dependency resolution that had to wait for a publish.
                // Collected rather than raised so finalisation, the orphan sweep and
                // docs all still run; reported once the pipeline has completed. ~keep
                let mut deferred_formatting: Vec<crate::e2e::format::DeferredFormatting> = Vec::new();

                // The binding-orphan sweep below needs last run's per-language output list as its
                // baseline. It cannot read that from `<lang>.manifest` (`cache::read_lang_manifest`):
                // `pipeline::generate` unconditionally overwrites that same file, for every language it
                // regenerates, via `write_lang_hash` a few lines down -- so by the time the sweep ran, a
                // "previous" read of `<lang>.manifest` was actually reading THIS run's own freshly
                // written output for any language that regenerated, and a path this run stopped emitting
                // could never appear there to be swept. `all-bindings-{lang}-ownership` is a dedicated
                // stage manifest `pipeline::generate` never touches, so reading it here -- before
                // `pipeline::generate` runs -- returns last run's binding list untouched. See
                // `binding_ownership`'s write-back below the sweep for the other half. ~keep
                let previous_binding_ownership: std::collections::HashMap<crate::core::config::Language, Vec<PathBuf>> =
                    languages
                        .iter()
                        .map(|language| {
                            (
                                *language,
                                cache::read_stage_paths(
                                    &resolved_cfg.name,
                                    &format!("all-bindings-{language}-ownership"),
                                ),
                            )
                        })
                        .collect();

                tracing::info!("Generating bindings...");
                let bindings = pipeline::generate(&api, resolved_cfg, &languages, clean, config_path, true)?;
                // `<lang>.manifest` (`cache::write_lang_manifest`) must hold the union of every
                // phase's output, not just this one: `pipeline::generate`'s own `write_lang_hash`
                // call already stamped it with only `bindings`, and that stays uncorrected for a
                // language `pipeline::generate` skips as lang-hash-cached (absent from `bindings`
                // entirely) unless seeded here from last run's own manifest -- mirrors
                // `alef generate`'s `language_output_paths` seeding in `core_commands.rs`. ~keep
                let regenerated_languages: std::collections::HashSet<_> =
                    bindings.iter().map(|(language, _)| *language).collect();
                let mut language_output_paths: std::collections::HashMap<
                    crate::core::config::Language,
                    std::collections::HashSet<PathBuf>,
                > = std::collections::HashMap::new();
                for language in languages
                    .iter()
                    .filter(|language| !regenerated_languages.contains(language))
                {
                    language_output_paths
                        .entry(*language)
                        .or_default()
                        .extend(cache::read_lang_manifest(&resolved_cfg.name, &language.to_string()));
                }
                // This run's per-language binding ownership: the exact file list `pipeline::generate`
                // just produced for every language it regenerated, plus -- unchanged -- last run's
                // recorded list for any language `pipeline::generate` skipped as cached (it is present
                // as a key here iff `pipeline::generate` regenerated it, even if that produced zero
                // files). A cache hit must not be read as "this language emitted nothing", or the sweep
                // below would delete every file a cached, unregenerated language still legitimately
                // owns. ~keep
                let mut binding_ownership: std::collections::HashMap<crate::core::config::Language, Vec<PathBuf>> =
                    bindings
                        .iter()
                        .map(|(language, generated)| {
                            (
                                *language,
                                generated.iter().map(|file| base_dir.join(&file.path)).collect(),
                            )
                        })
                        .collect();
                for language in languages.iter() {
                    if binding_ownership.contains_key(language) {
                        continue;
                    }
                    binding_ownership.insert(
                        *language,
                        previous_binding_ownership.get(language).cloned().unwrap_or_default(),
                    );
                }
                // `binding_ownership`'s cache-hit entries just above exist to answer exactly the
                // question the sweep below asks, but the sweep's `keep` set is `current_gen_paths`,
                // not `binding_ownership` -- and `current_gen_paths` is populated only from what
                // `bindings`/`stubs`/`public_api_files` actually returned this run, so a language
                // `pipeline::generate` skipped as cache-hit counted as having emitted nothing. The
                // manifest-based route in `sweep_manifest_orphans` (unlike its disk-scan route) has
                // no per-root "nothing recorded here" guard, so it deleted that language's still-valid
                // binding output on every cache hit -- `<lang>.manifest` stayed intact and non-empty,
                // but a file it named was gone, so the next run's `outputs_exist` read that as a miss,
                // regenerated, and the following hit deleted it again: an unbroken hit/miss/hit/miss
                // cycle (alef-tasks#303). Folding `binding_ownership` in here closes that gap. ~keep
                current_gen_paths.extend(binding_ownership.values().flatten().cloned());

                let mut binding_count: usize = 0;
                for (lang, lang_files) in &bindings {
                    let lang_str = lang.to_string();

                    current_gen_paths.extend(pipeline::stampable_output_paths(lang_files, &base_dir));
                    language_output_paths
                        .entry(*lang)
                        .or_default()
                        .extend(pipeline::managed_output_paths(lang_files, &base_dir));

                    let hashes: Vec<(String, String)> = lang_files
                        .iter()
                        .map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                        .collect();

                    let cache_key = format!("{}.{lang_str}", resolved_cfg.name);
                    let stored = cache::read_generation_hashes(&cache_key).unwrap_or_default();
                    let cache_match = !hashes.is_empty() && hashes.iter().all(|(p, h)| stored.get(p) == Some(h));

                    if cache_match && !clean && generated_files_match_disk(lang_files, &base_dir) {
                        tracing::info!("  [{lang_str}] up to date (skipping)");
                        continue;
                    }

                    let single = vec![(*lang, lang_files.clone())];
                    let report = pipeline::write_files_report(&single, &base_dir)?;
                    refusals.absorb_refusals(&report);
                    binding_count += report.changed_count();
                    if report.changed_count() > 0 {
                        any_output_changed = true;
                    }
                    let _ = cache::write_generation_hashes(&cache_key, &hashes);
                }

                if !api.services.is_empty() {
                    let svc_files = pipeline::generate_service_api(&api, resolved_cfg, &languages)?;
                    if !svc_files.is_empty() {
                        for (lang, files) in &svc_files {
                            current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                            language_output_paths
                                .entry(*lang)
                                .or_default()
                                .extend(pipeline::managed_output_paths(files, &base_dir));
                        }
                        let report = pipeline::write_files_report(&svc_files, &base_dir)?;
                        refusals.absorb_refusals(&report);
                        let svc_count = report.changed_count();
                        tracing::info!("Generated {svc_count} service API files");
                        if svc_count > 0 {
                            any_output_changed = true;
                        }
                    }
                }

                tracing::info!("Generating scaffolding...");
                // `alef all` always resolves the crate's full configured language set (there is
                // no `--lang` filter on this command), so the crate-wide scaffold manifest below
                // is always written from a complete file list and never clobbers another
                // language's recorded paths. See `write_scaffold_manifest`'s doc for why a
                // `--lang`-filtered caller must not call it. ~keep
                let previous_scaffold_paths = cache::read_scaffold_manifest(&resolved_cfg.name);
                let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
                // The stage that actually holds create-once seeds: `packages/php/composer.json`,
                // `crates/*-node/package.json`, `packages/java/pom.xml`, `packages/zig/build.zig`
                // and every placeholder test file are emitted `generated_header: false`, so
                // `can_skip` is the only thing between a hand-grown one and this run's
                // placeholder. See `create_once_overwrite` for why `clean` no longer answers
                // that question. ~keep
                let scaffold_report =
                    pipeline::write_scaffold_files_report(&scaffold_files, &base_dir, overwrite_create_once)?;
                refusals.absorb_refusals(&scaffold_report);
                let scaffold_count = scaffold_report.changed_count();
                if scaffold_count > 0 {
                    any_output_changed = true;
                }
                let scaffold_output_paths: Vec<PathBuf> =
                    scaffold_files.iter().map(|file| base_dir.join(&file.path)).collect();
                current_gen_paths.extend(pipeline::stampable_output_paths(&scaffold_files, &base_dir));
                let scaffold_keep: std::collections::HashSet<PathBuf> = scaffold_output_paths.iter().cloned().collect();
                let scaffold_sweep_roots = pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir);
                pipeline::sweep_manifest_orphans(&previous_scaffold_paths, &scaffold_keep, &scaffold_sweep_roots, &[])?;
                cache::write_scaffold_manifest(&resolved_cfg.name, &scaffold_output_paths)?;

                tracing::info!("Running post-build processing...");
                // A bare `?`/`return Err` here used to hard-stop the entire run: not just the
                // rest of THIS crate's stages (stubs, scaffold, e2e, docs never ran), but every
                // crate listed after this one in `crates_to_process` too -- one backend's
                // post-build failure (e.g. a Dart `flutter_rust_bridge_codegen` break) denied a
                // multi-crate workspace any regeneration at all (task #186). Deferred into
                // `stage_failures` instead: this crate's remaining stages, and every later
                // crate, still run. `refusals` is still reported -- once, at the very end of
                // this function, covering this failure alongside every other write refusal --
                // which is what turns "install/enable flutter_rust_bridge_codegen" (misleading;
                // the tool is present) into "run `alef adopt <path>`" (the actual fix). ~keep
                if let Err(error) = complete_generated_artifacts(&languages, resolved_cfg, &base_dir) {
                    stage_failures.record(&format!("[{}] post-build processing", resolved_cfg.name), error);
                }

                tracing::info!("Generating type stubs...");
                let stubs = pipeline::generate_stubs(&api, resolved_cfg, &languages)?;

                let stub_hashes: Vec<(String, String)> = stubs
                    .iter()
                    .flat_map(|(_, fs)| {
                        fs.iter().map(|f| {
                            (
                                base_dir.join(&f.path).display().to_string(),
                                cache::hash_content(&f.content),
                            )
                        })
                    })
                    .collect();
                let stubs_cache_key = format!("{}.stubs", resolved_cfg.name);
                let stored_stubs = cache::read_generation_hashes(&stubs_cache_key).unwrap_or_default();
                let stubs_match =
                    !stub_hashes.is_empty() && stub_hashes.iter().all(|(p, h)| stored_stubs.get(p) == Some(h));

                let stub_count = if !stubs_match || clean {
                    let report = pipeline::write_files_report(&stubs, &base_dir)?;
                    refusals.absorb_refusals(&report);
                    let count = report.changed_count();
                    let _ = cache::write_generation_hashes(&stubs_cache_key, &stub_hashes);
                    if count > 0 {
                        any_output_changed = true;
                    }
                    count
                } else {
                    tracing::info!("  [stubs] up to date (skipping)");
                    0
                };

                for (lang, files) in &stubs {
                    current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                    language_output_paths
                        .entry(*lang)
                        .or_default()
                        .extend(pipeline::managed_output_paths(files, &base_dir));
                }

                let mut api_count = 0;
                if resolved_cfg.generate.public_api {
                    let public_api_files = pipeline::generate_public_api(&api, resolved_cfg, &languages, config_path)?;
                    if !public_api_files.is_empty() {
                        let api_hashes: Vec<(String, String)> = public_api_files
                            .iter()
                            .flat_map(|(_, fs)| {
                                fs.iter().map(|f| {
                                    let normalized = pipeline::normalize_content(&f.path, &f.content);
                                    (
                                        base_dir.join(&f.path).display().to_string(),
                                        cache::hash_content(&normalized),
                                    )
                                })
                            })
                            .collect();
                        let api_cache_key = format!("{}.public_api", resolved_cfg.name);
                        let stored_api = cache::read_generation_hashes(&api_cache_key).unwrap_or_default();
                        let api_match =
                            !api_hashes.is_empty() && api_hashes.iter().all(|(p, h)| stored_api.get(p) == Some(h));

                        for (lang, files) in &public_api_files {
                            current_gen_paths.extend(pipeline::stampable_output_paths(files, &base_dir));
                            language_output_paths
                                .entry(*lang)
                                .or_default()
                                .extend(pipeline::managed_output_paths(files, &base_dir));
                        }

                        if !api_match || clean {
                            let report = pipeline::write_files_report(&public_api_files, &base_dir)?;
                            refusals.absorb_refusals(&report);
                            api_count = report.changed_count();
                            tracing::info!("Generated {api_count} public API files");
                            if api_count > 0 {
                                any_output_changed = true;
                            }
                            let _ = cache::write_generation_hashes(&api_cache_key, &api_hashes);
                        } else {
                            tracing::info!("  [public_api] up to date (skipping)");
                        }
                    }
                }

                if !api.version.is_empty() {
                    let pkg = base_dir.join("Package.swift");
                    if let Ok(content) = std::fs::read_to_string(&pkg) {
                        let updated = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{}", api.version));
                        if updated != content {
                            std::fs::write(&pkg, updated)?;
                        }
                    }
                }

                let mut e2e_count = 0;
                if let Some(e2e_config) = &resolved_cfg.e2e {
                    let all_calls = std::iter::once(("_default", &e2e_config.call))
                        .chain(e2e_config.calls.iter().map(|(k, v)| (k.as_str(), v)));
                    for (call_name, call_config) in all_calls {
                        if call_config.function.is_empty() || call_config.module.is_empty() {
                            continue;
                        }
                        let module_path = call_config.module.replace('-', "_");
                        let function_name = &call_config.function;
                        match crate::extract::validate_call_export(&api, &module_path, function_name) {
                            crate::extract::ExportValidation::Ok => {}
                            crate::extract::ExportValidation::NotFound { function } => {
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' was not found in the extracted API surface. \
                                 Check that it is declared `pub` and that its source file is listed in \
                                 [[crate.sources]] or [[crate.source_crates]]."
                                );
                            }
                            crate::extract::ExportValidation::WrongPath {
                                function,
                                declared_module,
                                actual_paths,
                            } => {
                                let paths = actual_paths.join(", ");
                                anyhow::bail!(
                                    "e2e call '{call_name}': function '{function}' is not exported at module path \
                                 '{declared_module}' -- the Rust codegen would emit `use {declared_module}::{function};`. \
                                 Actual rust_path(s) found: {paths}. \
                                 Fix: either add `pub use <path>::{function};` at the crate root, \
                                 or update `module` in [e2e.calls.{call_name}] to the correct path."
                                );
                            }
                        }
                    }

                    let fixtures_dir = std::path::Path::new(&e2e_config.fixtures);
                    let fixture_hash = cache::hash_directory(fixtures_dir).unwrap_or_default();
                    let ir_json = serde_json::to_string(&api)?;
                    let e2e_stage_hash = cache::compute_stage_hash(&ir_json, "e2e", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "e2e", &e2e_stage_hash) {
                        tracing::info!("  [e2e] up to date (skipping)");
                        let cached_paths = cache::read_stage_paths(&resolved_cfg.name, "e2e");
                        deferred_formatting.extend(crate::e2e::format::run_formatters_for_cached_paths(
                            &cached_paths,
                            &base_dir,
                            e2e_config,
                            strict,
                        )?);
                        for path in cached_paths {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        tracing::info!("Generating e2e test suites...");
                        let previous_paths = cache::read_stage_paths(&resolved_cfg.name, "e2e");
                        let (files, generator_error) = crate::e2e::generate_e2e(
                            resolved_cfg,
                            e2e_config,
                            None,
                            &api.types,
                            &api.enums,
                            &api.functions,
                            &api.errors,
                        )?;
                        let e2e_report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        refusals.absorb_refusals(&e2e_report);
                        e2e_count = e2e_report.changed_count();
                        if e2e_count > 0 {
                            any_output_changed = true;
                        }
                        let managed_files: Vec<_> = files
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .cloned()
                            .collect();
                        deferred_formatting.extend(crate::e2e::format::run_formatters(
                            &managed_files,
                            e2e_config,
                            strict,
                        )?);

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        // The one pre-`format_generated_reporting` stamp this loop keeps, and the
                        // only one that is genuinely post-format: `run_formatters` immediately
                        // above is this subtree's own formatting pass. ~keep
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        // A generator failure here must not reach either line below -- see
                        // `e2e_stage_error`'s doc comment above the loop for the two-part hazard
                        // (cache poisoning that hides the failure next run, and orphan-sweeping the
                        // last known-good backend output). Write, format and hash finalisation above
                        // still ran unconditionally, so this run's partial output still carries a
                        // provenance marker for the ownership guard. ~keep
                        if let Some(error) = generator_error {
                            if e2e_stage_error.is_some() {
                                tracing::error!("[{}] e2e codegen failed: {error:#}", resolved_cfg.name);
                            }
                            e2e_stage_error.get_or_insert(error);
                        } else {
                            let e2e_output_root = base_dir.join(&e2e_config.output);
                            pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &[e2e_output_root], &[])?;

                            cache::write_stage_hash(&resolved_cfg.name, "e2e", e2e_stage_hash.as_str(), &output_paths)?;
                        }

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }

                    let test_apps_stage_hash =
                        cache::compute_stage_hash(&ir_json, "test-apps", &config_toml, &fixture_hash);
                    if !clean && cache::is_stage_cached(&resolved_cfg.name, "test-apps", &test_apps_stage_hash) {
                        tracing::info!("  [test-apps] up to date (skipping)");
                        let cached_paths = cache::read_stage_paths(&resolved_cfg.name, "test-apps");
                        let mut registry_e2e_config = e2e_config.clone();
                        registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        deferred_formatting.extend(crate::e2e::format::run_formatters_for_cached_paths(
                            &cached_paths,
                            &base_dir,
                            &registry_e2e_config,
                            strict,
                        )?);
                        for path in cached_paths {
                            current_gen_paths.insert(path);
                        }
                    } else {
                        tracing::info!("Generating registry-mode test apps...");
                        let previous_paths = cache::read_stage_paths(&resolved_cfg.name, "test-apps");
                        let mut registry_e2e_config = e2e_config.clone();
                        registry_e2e_config.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
                        let registry_e2e_ref = &registry_e2e_config;

                        let (files, generator_error) = crate::e2e::generate_e2e(
                            resolved_cfg,
                            registry_e2e_ref,
                            None,
                            &api.types,
                            &api.enums,
                            &api.functions,
                            &api.errors,
                        )?;
                        let test_apps_report = pipeline::write_scaffold_files_report(&files, &base_dir, true)?;
                        refusals.absorb_refusals(&test_apps_report);
                        let test_apps_count = test_apps_report.changed_count();
                        e2e_count += test_apps_count;
                        if test_apps_count > 0 {
                            any_output_changed = true;
                        }
                        let managed_files: Vec<_> = files
                            .iter()
                            .filter(|file| file.carries_alef_marker())
                            .cloned()
                            .collect();
                        deferred_formatting.extend(crate::e2e::format::run_formatters(
                            &managed_files,
                            registry_e2e_ref,
                            strict,
                        )?);

                        let output_paths: Vec<PathBuf> = managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
                        let path_set: std::collections::HashSet<PathBuf> = output_paths.iter().cloned().collect();

                        // Kept for the same reason as the `e2e` stage's stamp above: it follows
                        // that subtree's own native formatting pass, not precedes it. ~keep
                        pipeline::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

                        // Same hazard, same gate, as the `e2e` stage above -- see `e2e_stage_error`'s
                        // doc comment above the loop. ~keep
                        if let Some(error) = generator_error {
                            if e2e_stage_error.is_some() {
                                tracing::error!("[{}] test-apps codegen failed: {error:#}", resolved_cfg.name);
                            }
                            e2e_stage_error.get_or_insert(error);
                        } else {
                            let test_apps_root = base_dir.join(registry_e2e_ref.effective_output());
                            pipeline::sweep_manifest_orphans(&previous_paths, &path_set, &[test_apps_root], &[])?;

                            cache::write_stage_hash(
                                &resolved_cfg.name,
                                "test-apps",
                                test_apps_stage_hash.as_str(),
                                &output_paths,
                            )?;
                        }

                        for path in output_paths {
                            current_gen_paths.insert(path);
                        }
                    }
                }

                tracing::info!("Generating READMEs...");
                let readme_languages = crate::readme::expand_configured_readme_languages(resolved_cfg, &languages);
                let readme_files = pipeline::readme(&api, resolved_cfg, &readme_languages)?;
                let readme_report = pipeline::write_scaffold_files_report(&readme_files, &base_dir, true)?;
                refusals.absorb_refusals(&readme_report);
                let readme_count = readme_report.changed_count();
                if readme_count > 0 {
                    any_output_changed = true;
                }
                current_gen_paths.extend(pipeline::stampable_output_paths(&readme_files, &base_dir));

                tracing::info!("Generating docs...");
                warn_if_snippet_validation_needs_build(resolved_cfg);
                let docs_api = pipeline::extract(resolved_cfg, config_path, false)?;
                let doc_languages = resolve_doc_languages(resolved_cfg, None)?;
                // `generate_docs_stage` hands back every page it rendered even when a later step
                // (snippet validation, CLI/MCP adoption, llms/skills) fails, specifically so a
                // strict-mode bail never discards already-rendered API reference pages. Write and
                // hash `doc_files` before propagating `doc_result`, not after. ~keep
                let (doc_files, doc_result) = docs_stage::generate(
                    skip_snippet_validation,
                    &docs_api,
                    resolved_cfg,
                    &doc_languages,
                    &base_dir,
                );
                // Inert today and kept honest on purpose: `docs::generate_docs_stage` forces
                // `generated_header = true` on every reference page and every extra
                // (`cli.md`, `mcp.md`, `llms.txt`, `SKILL.md`) it emits, so `can_skip` cannot
                // fire here whatever this argument says -- threading `clean` in was never
                // buying the docs stage anything. Passing the same decision as the scaffold
                // stage rather than a bare `true` means the day a docs page is emitted as a
                // seed, it is protected by default instead of silently clobbered. ~keep
                let doc_report = pipeline::write_scaffold_files_report(&doc_files, &base_dir, overwrite_create_once)?;
                refusals.absorb_refusals(&doc_report);
                let doc_count = doc_report.changed_count();
                if doc_count > 0 {
                    any_output_changed = true;
                }
                current_gen_paths.extend(pipeline::stampable_output_paths(&doc_files, &base_dir));
                // Snippet/doc validation (`docs::generate_docs_stage`'s later sub-steps) reads its
                // input from disk, not from `doc_files` in memory. When the ownership guard refuses
                // a write earlier in this same run -- e.g. a pre-marker-fix snippet with no durable
                // ownership record -- the file on disk stays exactly as stale as it was before this
                // run started, and a validation failure against it reads as a defect in freshly
                // generated content when it is actually a defect in content this run never touched.
                // Naming the refusal count on the error, right where it surfaces, is what makes that
                // distinguishable without cross-referencing a warning log emitted stages earlier.
                //
                // A failure here is deferred, not returned: the formatting/sweep/hash-finalisation/
                // hook-installation steps below must still run for this crate (and this loop must
                // still reach every later crate) even though its docs stage failed -- see
                // `docs_stage_error`'s doc comment above the loop for why. `doc_result` is matched by
                // value instead of re-testing `.is_err()` after this point, because the `Ok` arm right
                // below still needs the snippet-refusal warning to run exactly once. ~keep
                match doc_result {
                    Ok(()) => {
                        // An `Ok` snippet-validation verdict is not proof the validated content came
                        // from this run. Same disk-read hazard as the `Err` arm above, just silent
                        // instead of loud: a refused write inside `docs.snippets.dirs`/`inline_dirs`
                        // leaves pre-run bytes in place for `discover_snippets` to grade, and a
                        // validator that happens to accept those stale bytes reports success with no
                        // trace that this run never produced what it graded. That is the "refused
                        // 2,897 writes, reported success, validated two-day-old content" failure mode
                        // -- attribute it here the same way the `Err` arm does. ~keep
                        let snippet_refusals =
                            refused_snippet_dir_paths(&refusals.refused_paths, resolved_cfg, &base_dir);
                        if !snippet_refusals.is_empty() {
                            tracing::warn!(
                                "[{}] docs/snippet validation passed, but {} write(s) inside its \
                                 docs.snippets root(s) were refused by the ownership guard this run -- \
                                 validation graded stale, pre-run content at those paths, not anything \
                                 this run rendered: {}",
                                resolved_cfg.name,
                                snippet_refusals.len(),
                                snippet_refusals
                                    .iter()
                                    .map(|path| path.display().to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        }
                    }
                    Err(error) => {
                        // Gated on `refused_snippet_dir_paths` (refusals inside `docs.snippets`
                        // roots), not `refusals.refused_count()` (every refusal anywhere in the
                        // run): a refusal to an unrelated scaffold or README file must not attach
                        // an "ownership guard" excuse to a validation failure it had nothing to do
                        // with -- that wrong attribution is what previously sent investigators
                        // chasing the ownership guard for a plain checkstyle/compiler defect in
                        // freshly generated content. Mirrors the `Ok` arm above: both arms consult
                        // the same scoped set so a validation failure and a validation pass
                        // attribute refusals identically. ~keep
                        let snippet_refusals =
                            refused_snippet_dir_paths(&refusals.refused_paths, resolved_cfg, &base_dir);
                        let error = if !snippet_refusals.is_empty() {
                            pipeline::report_refused_writes(&refusals);
                            error.context(format!(
                                "{} file write(s) inside this crate's docs.snippets root(s) were refused by \
                                 the ownership guard this run (see the refusal report above). \
                                 Docs/snippet validation reads content from disk, so a refused write \
                                 leaves stale content in place for it to grade -- if this failure looks \
                                 like a content mismatch rather than a real defect, check whether the \
                                 affected path is among the refused writes and run `alef adopt <path>`.",
                                snippet_refusals.len()
                            ))
                        } else {
                            error
                        };
                        if docs_stage_error.is_some() {
                            // A second (or later) crate's docs failure in the same multi-crate run.
                            // Only one error becomes this function's `Result`; without this, every
                            // failure past the first would vanish with no trace at all. ~keep
                            tracing::error!("[{}] docs/snippet validation failed: {error:#}", resolved_cfg.name);
                        }
                        docs_stage_error.get_or_insert(error);
                    }
                }

                let cleanup_roots = pipeline::generate_sweep_roots(&languages, false, resolved_cfg, &base_dir);
                // `previous_binding_ownership` (read above, before `pipeline::generate` could ever
                // overwrite `<lang>.manifest`) is the correct baseline -- see its doc comment for why
                // `read_lang_manifest` cannot be used here. `binding_ownership` is written back as the
                // new baseline only now, after the sweep has consumed the old one, mirroring
                // `generate-{language}-ownership`'s read-before / write-after ordering in
                // `alef generate` (`bin_cli/core_commands.rs`). Kept as its own dedicated stage rather
                // than sharing that exact stage name: `generate-{language}-ownership` also folds in
                // service-API, stub and public-API paths that `alef all` tracks and sweeps separately
                // (or not at all here), so writing this narrower, bindings-only list under the shared
                // name would let `alef all` silently truncate the broader baseline `alef generate`
                // relies on the next time the two commands are run back to back. ~keep
                let previous_paths: Vec<_> = previous_binding_ownership.into_values().flatten().collect();
                // `cleanup_roots` doubles as the disk-scan candidate list -- see the matching
                // comment at the `alef generate` call site (`core_commands.rs`) for why this is
                // safe: `sweep_manifest_orphans` only scans a root once it independently confirms
                // both `previous_paths` and `current_gen_paths` carry an entry under it. That
                // per-root check is what keeps a language `pipeline::generate`'s per-language cache
                // skipped this run -- which leaves `current_gen_paths` with zero entries under that
                // language's root, not merely a stale one -- from being scanned at all. ~keep
                pipeline::sweep_manifest_orphans(&previous_paths, &current_gen_paths, &cleanup_roots, &cleanup_roots)?;
                for (language, paths) in &binding_ownership {
                    cache::write_stage_hash(
                        &resolved_cfg.name,
                        &format!("all-bindings-{language}-ownership"),
                        &sources_hash,
                        paths,
                    )?;
                }
                // Replaces the bindings-only manifest `pipeline::generate`'s own `write_lang_hash`
                // call left behind, now that every phase (bindings, service API, stubs, public API)
                // has contributed its `carries_alef_marker()` paths to `language_output_paths` above.
                // Writing this after the sweep (not before) matches the ownership write-back just
                // above: both are end-of-loop bookkeeping the sweep must not observe as this run's
                // "previous" state. ~keep
                for (language, paths) in &language_output_paths {
                    let paths: Vec<_> = paths.iter().cloned().collect();
                    cache::write_lang_manifest(&resolved_cfg.name, &language.to_string(), &paths)?;
                }

                // `any_output_changed` alone is the wrong gate for a tree an EARLIER alef stamped
                // without ever formatting: nothing was written this run, and the tree is still
                // non-canonical. `generated_tree_needs_formatting` answers "no" on a settled tree,
                // so the fast path is unchanged -- see `pipeline::format::stamp_gate`. ~keep
                if any_output_changed || pipeline::generated_tree_needs_formatting(&base_dir) {
                    tracing::info!("Formatting generated files...");
                    // Scope-symmetric by construction: `finalize_hashes_sweeping` below re-stamps
                    // `current_gen_paths` plus everything under `cleanup_roots`, a superset of what
                    // is stripped here, so no file can be left carrying no hash line at all. ~keep
                    let unstamped = pipeline::unstamp_before_formatting(&current_gen_paths);
                    tracing::debug!("unstamped {unstamped} generated file(s) so the formatter can see them");
                    // `None` selects the converging whole-tree pass, which is what a full regen needs
                    // and what `converge_full_regen_formatting` documents itself as serving. Passing
                    // `Some(&only_languages_that_wrote_bindings)` would take the single-pass branch
                    // instead, so the loop that exists precisely because poly's .cs/.java/.json engines
                    // are not single-pass idempotent would never run on the one command that regenerates
                    // everything: `alef all` would leave drift that a second `alef all` would silently
                    // settle, and stamp hashes over it. The language filter is also wrong for the
                    // workspace-wide `cargo sort -n -w` folded into that loop, which must cover crates
                    // this run did not generate. ~keep
                    // `strict` must reach this pass too: until it did, `--strict` guarded the e2e
                    // tree while `packages/<lang>` -- the shipped bindings -- went unguarded. ~keep
                    let skipped = pipeline::format_generated_reporting(resolved_cfg, &base_dir, None, strict)?;
                    deferred_formatting.extend(skipped);
                }

                tracing::info!("Finalising hashes...");
                // Sweeping (not the plain path-tracked `finalize_hashes` used by the
                // earlier per-stage checkpoints above) so that a language dropped from
                // `bindings` by the per-language cache in `pipeline::generate` -- and
                // therefore never added to `current_gen_paths` -- still gets its
                // on-disk output re-stamped from `cleanup_roots`. Safe to run after
                // `sweep_manifest_orphans` above: it clones `current_gen_paths` rather
                // than mutating it, so the orphan sweep already saw the untouched,
                // precisely-tracked set.
                pipeline::finalize_hashes_sweeping(
                    &current_gen_paths,
                    &cleanup_roots,
                    &sources_hash,
                    &alef_toml_bytes,
                )?;

                // Reported only now, after finalisation, the orphan sweep and docs have
                // all run. Raising at the point of failure is what made the release
                // unreachable: these steps resolve the very version the run produces. ~keep
                report_deferred_formatting(&resolved_cfg.name, &deferred_formatting);

                grand_binding_count += binding_count;
                grand_stub_count += stub_count;
                grand_api_count += api_count;
                grand_scaffold_count += scaffold_count;
                grand_readme_count += readme_count;
                grand_e2e_count += e2e_count;
                grand_doc_count += doc_count;
            }

            pipeline::install_poly_hooks(&base_dir);

            pipeline::report_refused_writes(&refusals);
            tracing::info!(
                "Done: {grand_binding_count} binding files, {grand_stub_count} stub files, {grand_api_count} API files, {grand_scaffold_count} scaffold files, {grand_readme_count} readme files, {grand_e2e_count} e2e files, {grand_doc_count} doc files"
            );
            // Folded last, after every crate has been through formatting, orphan sweeping, hash
            // finalisation and hook installation -- see `docs_stage_error`'s doc comment for why a
            // docs/snippet validation failure must not reach this point any earlier than this. The
            // run still exits non-zero, and the error's own context (including the refusal-count
            // wrapping above) is untouched; only the timing of the `return` moved. Both go into
            // `stage_failures` rather than an early `return Err`, alongside any pre-flight or
            // post-build failure recorded earlier: a run that hit more than one of these categories
            // used to report only `e2e_stage_error` and silently drop `docs_stage_error` entirely --
            // this is the "accurate summary naming everything that went wrong" task #186 asks for,
            // not just the first thing. ~keep
            if let Some(error) = e2e_stage_error {
                stage_failures.record("e2e codegen", error);
            }
            if let Some(error) = docs_stage_error {
                stage_failures.record("docs/snippet validation", error);
            }
            stage_failures.into_result()?;
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}

#[cfg(test)]
#[path = "all_commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "all_commands_refusal_tests.rs"]
mod refusal_tests;

#[cfg(test)]
#[path = "all_commands_defer_tests.rs"]
mod defer_tests;
