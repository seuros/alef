//! `alef verify` -- the read-only staleness, completeness, and drift report.
//!
//! Split out of `core_commands.rs` rather than added to it: that file is over the
//! 1,000-line cap this repository sets for backend/codegen/CLI sources, and `verify` is a
//! self-contained concern (it shares nothing with the other command arms but the helper
//! module every arm uses). ~keep

use anyhow::Result;

use crate::cli::{cache, dispatch, pipeline};

use super::super::args::Commands;
use super::super::dispatch::DispatchContext;
use super::super::helpers::*;
use super::super::verify_orphans;

/// Run `alef verify`.
///
/// # Errors
///
/// Returns an error when configuration or extraction fails, and -- unless `report_only` --
/// when the verification itself finds drift.
pub(super) fn run(context: &DispatchContext, report_only: bool) -> Result<Option<Commands>> {
    let config_path = &context.config_path;
    let (_workspace, resolved) = load_config(config_path)?;
    let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;
    // Not "inputs-hash mode": the embedded per-file hash folds in the file's own
    // content (see `core::hash`'s module doc), so this also catches hand-edited
    // or reverted generated output, not only stale generation inputs. ~keep
    tracing::info!("Verifying alef-generated files (per-file inputs+content hash)");
    let base_dir = std::env::current_dir()?;

    let missing_snippet_roots: Vec<String> = crates_to_process
        .iter()
        .flat_map(|resolved_cfg| missing_snippet_directories(resolved_cfg, &base_dir))
        .collect();
    let has_missing_snippet_roots = !missing_snippet_roots.is_empty();

    let alef_toml_bytes = cache::read_alef_toml_bytes(config_path);

    let all_inputs_hashes: Vec<String> = crates_to_process
        .iter()
        .filter_map(|c| cache::sources_hash(&c.sources).ok())
        .map(|sh| crate::core::hash::compute_inputs_hash(&sh, &alef_toml_bytes))
        .collect();

    let stale = verify_walk_multi(&base_dir, &all_inputs_hashes)?;

    let mut snippet_coverage_issues = Vec::new();
    // `verify_walk_multi` only sees files that already exist on disk; a file
    // generation would now produce but that was never written (a backend
    // that emits one file per public type, an item added since the last
    // regen) is invisible to it. Closing that requires knowing what
    // generation would produce, so every crate pays a regeneration pass
    // here (mirrors `alef diff`) to find files entirely absent from disk, and
    // -- in the same pass -- files that exist but were never marked and so
    // can never be written by a plain `alef generate` (frozen; see
    // `FrozenFile`). ~keep
    let mut missing_generated_files: Vec<String> = Vec::new();
    // Absent AND gitignored: `alef generate` cannot close this gap the way it closes a
    // plain `missing_generated_files` entry -- see `MissingAndFrozenFiles::missing_gitignored`. ~keep
    let mut missing_gitignored_generated_files: Vec<String> = Vec::new();
    let mut frozen_generated_files: Vec<FrozenFile> = Vec::new();
    // Unioned across every crate before the orphan diff runs below: a file legitimately
    // owned by crate B must never look orphaned merely because crate A's own managed
    // surface doesn't mention it. See `verify_orphans::find_orphaned_generated_files`. ~keep
    let mut all_managed_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    // Debt `collect_managed_surface` tolerated while still building the rest of
    // the surface (currently only the e2e stages' deferred strict-assertion
    // failure). `alef verify` is read-only and has no target to excuse a stage
    // failure the way `alef adopt` can, so every one of these is collected and
    // reported below rather than silently absorbed into a clean-looking zero --
    // see `collect_managed_surface`'s doc for why dropping this list is exactly
    // the bug this return shape exists to prevent. ~keep
    let mut stage_failures: Vec<String> = Vec::new();
    // Informational only -- see `pipeline::generate::scaffold_drift`'s module doc for what
    // this checks, its stated false-positive/false-negative shape, and why it never
    // contributes to `has_stage_failures` or any other hard-fail condition below. ~keep
    let mut create_once_template_drift: Vec<String> = Vec::new();
    for resolved_cfg in &crates_to_process {
        let languages = resolve_languages(resolved_cfg, None)?;
        let api = pipeline::extract(resolved_cfg, config_path, false)?;
        let scaffold_files = pipeline::scaffold(&api, resolved_cfg, &languages, config_path)?;
        create_once_template_drift.extend(
            pipeline::find_create_once_template_drift(&scaffold_files, &base_dir)
                .into_iter()
                .map(|path| format!("[{}] {}", resolved_cfg.name, path.display())),
        );
        let found = find_missing_and_frozen_generated_files(&languages, &api, resolved_cfg, config_path, &base_dir)?;
        missing_generated_files.extend(found.missing);
        missing_gitignored_generated_files.extend(found.missing_gitignored);
        frozen_generated_files.extend(found.frozen);
        all_managed_paths.extend(found.managed_paths);
        stage_failures.extend(
            found
                .stage_failures
                .into_iter()
                .map(|failure| format!("[{}] {failure}", resolved_cfg.name)),
        );

        let Some(e2e_config) = &resolved_cfg.e2e else {
            continue;
        };
        if let Err(error) = crate::e2e::verify_fresh_snippet_coverage(
            &base_dir,
            resolved_cfg,
            e2e_config,
            &api.types,
            &api.enums,
            &api.functions,
        ) {
            snippet_coverage_issues.push(format!("[{}] {error:#}", resolved_cfg.name));
        }
    }
    missing_generated_files.sort();
    missing_generated_files.dedup();
    missing_gitignored_generated_files.sort();
    missing_gitignored_generated_files.dedup();
    frozen_generated_files.sort_by(|a, b| a.path.cmp(&b.path));
    frozen_generated_files.dedup_by(|a, b| a.path == b.path);
    stage_failures.sort();
    stage_failures.dedup();
    create_once_template_drift.sort();
    create_once_template_drift.dedup();
    let has_stage_failures = !stage_failures.is_empty();
    let has_missing_files = !missing_generated_files.is_empty();
    let has_missing_gitignored_files = !missing_gitignored_generated_files.is_empty();
    let has_frozen_files = !frozen_generated_files.is_empty();
    // Only a frozen file that `alef adopt --write` will actually ACCEPT may gate the exit code.
    // A create-once seed's missing marker is deliberate, not drift: the write guard refuses it by
    // design, a plain `alef generate` leaves it untouched, and adopting it needs the deliberate
    // `--clobber-create-once-seeds`. Counting those made `alef verify` unable to reach exit 0 on
    // repos carrying legacy pre-marker files -- no amount of regeneration cleared them -- which
    // turned the release gate into something operators had to route around with a dangerous flag.
    // `create_once_template_drift` is already informational-only for the same reason. ~keep
    let has_adoptable_frozen_files =
        crate::bin_cli::helpers::frozen::has_adoptable_frozen_files(&frozen_generated_files);
    // Report-only: see `verify_orphans`'s module doc for why this never deletes.
    log_managed_surface(&all_managed_paths);
    let orphan_generated_files = verify_orphans::find_orphaned_generated_files(&base_dir, &all_managed_paths);
    let has_orphan_files = !orphan_generated_files.is_empty();

    // Catches the cross-artifact ABI straddle a per-file hash check cannot
    // see: an FFI header and a binding backend's opaque-handle file each
    // individually fresh against current inputs, but stamped by two
    // different handle-ABI generations because only one side was
    // regenerated. See `crate::core::hash::HANDLE_ABI_STAMP_KEY` and
    // `find_stamp_disagreement` for why 0/1 distinct values is silently
    // fine and only 2+ is reported. ~keep
    let abi_disagreement = find_stamp_disagreement(&base_dir, crate::core::hash::HANDLE_ABI_STAMP_KEY);
    let has_abi_disagreement = abi_disagreement.is_some();
    if let Some(disagreement) = &abi_disagreement {
        crate::bin_cli::output::line(format_args!(
            "ABI generation disagreement detected for `{}`:",
            disagreement.key
        ));
        for (path, value) in &disagreement.examples {
            crate::bin_cli::output::line(format_args!("  {path} -> {value}"));
        }
    }

    let mut all_version_mismatches: Vec<String> = Vec::new();
    for resolved_cfg in &crates_to_process {
        let mismatches = pipeline::verify_versions(resolved_cfg)?;
        all_version_mismatches.extend(mismatches);
    }
    let has_version_issues = !all_version_mismatches.is_empty();
    if has_version_issues {
        crate::bin_cli::output::line("Version mismatches detected:");
        for mismatch in &all_version_mismatches {
            crate::bin_cli::output::line(format_args!("  {mismatch}"));
        }
    }
    if !snippet_coverage_issues.is_empty() {
        crate::bin_cli::output::line("Snippet coverage issues detected:");
        for issue in &snippet_coverage_issues {
            crate::bin_cli::output::line(format_args!("  {issue}"));
        }
    }
    // Deliberately not folded into `snippet_coverage_issues` above: that list is about
    // generated-snippet coverage ledgers being fresh, an entirely different question from
    // whether the roots naming them exist at all. ~keep
    if has_missing_snippet_roots {
        crate::bin_cli::output::line(
            "Configured docs.snippets roots that do not exist (every snippet check that walks \
             these passes having examined nothing -- fix the dirs/inline_dirs entry or create \
             the directory):",
        );
        for directory in &missing_snippet_roots {
            crate::bin_cli::output::line(format_args!("  {directory}"));
        }
    }

    // Informational only, printed unconditionally and never folded into the "up to
    // date"/failure gates below: see `pipeline::generate::scaffold_drift`'s module doc.
    // A create-once file differing from its template is the expected steady state for a
    // hand-maintained file -- this only fires when the file's own git history rules out a
    // consumer edit as the explanation, and even then there is no rerun that fixes it;
    // only a human reviewing the current template can decide what to do. ~keep
    if !create_once_template_drift.is_empty() {
        crate::bin_cli::output::line(
            "Create-once scaffold files that predate a template fix (informational -- these are \
             user-owned after their first write, so alef never rewrites them; review the current \
             template and hand-port the fix if it applies to your copy):",
        );
        for path in &create_once_template_drift {
            crate::bin_cli::output::line(format_args!("  {path}"));
        }
    }

    // The `verify` half of the escalation `cache::untracked_required_records`
    // documents: write commands warn and keep going, verification must refuse. The
    // query is already silent outside a git work tree and for a record that does not
    // exist yet, so this never fires where "untracked" is unanswerable, nor on the
    // run that legitimately creates the record. ~keep
    let untracked_records = cache::untracked_required_records(&base_dir);
    if !untracked_records.is_empty() {
        crate::bin_cli::output::line(
            "Required alef records are not tracked by git (alef writes these and depends on them \
         being committed):",
        );
        for record in &untracked_records {
            crate::bin_cli::output::line(format_args!("  {record} -- fix with: git add {record}"));
        }
    }

    if stale.is_empty()
        && !has_missing_files
        && !has_missing_gitignored_files
        && !has_adoptable_frozen_files
        && !has_orphan_files
        && !has_abi_disagreement
        && !has_version_issues
        && snippet_coverage_issues.is_empty()
        && untracked_records.is_empty()
        && !has_stage_failures
        && !has_missing_snippet_roots
    {
        crate::bin_cli::output::line("All bindings and versions are up to date.");
    } else {
        if !stale.is_empty() {
            crate::bin_cli::output::line("Stale bindings detected:");
            for s in &stale {
                crate::bin_cli::output::line(format_args!("  {}", s.path));
                if tracing::enabled!(tracing::Level::DEBUG) {
                    crate::bin_cli::output::line(format_args!("    embedded:  {}", s.embedded));
                    let computed_str = s.computed.join(", ");
                    crate::bin_cli::output::line(format_args!("    computed:  {computed_str}"));
                }
            }
        }
        if has_missing_files {
            crate::bin_cli::output::line("Missing generated files detected:");
            for path in &missing_generated_files {
                crate::bin_cli::output::line(format_args!("  {path}"));
            }
        }
        // Distinct from `has_missing_files` above on purpose: `alef generate` is not a
        // remedy here, it is the failure mode -- the file gets written, then discarded by
        // the ignore rule before it can ever be committed, and the next `alef verify` finds
        // it "missing" again. Naming the correct fix (narrow the .gitignore rule, then
        // commit) is the entire point of splitting this out from plain "missing" instead of
        // folding it into the same heading with the same generate-and-rerun remedy. ~keep
        if has_missing_gitignored_files {
            crate::bin_cli::output::line(
                "Missing generated files that are also gitignored detected (running `alef generate` \
             cannot fix these -- the file would be written, then discarded by the matching \
             .gitignore rule before it can be committed; narrow the ignore rule for each path \
             below, then commit the file):",
            );
            for path in &missing_gitignored_generated_files {
                crate::bin_cli::output::line(format_args!("  {path}"));
            }
        }
        // Reported separately from stale/missing, never folded into either
        // count: the remedy is different (a human must review and adopt or
        // delete the file -- `alef generate` alone cannot fix it) and folding
        // it in would make a frozen file look like ordinary drift. ~keep
        //
        // Split into two headings by `create_once`, never one: a create-once seed and a
        // genuinely adoptable frozen file need different `alef adopt` invocations, and
        // printing them under one heading with one remedy is the exact defect this split
        // closes -- `alef adopt --converged-only --write` refused every single frozen
        // path it was pointed at in two consumer repos (85 of 85 in one, 99 of 99 in the
        // other) because every one of them was a create-once seed and the old, singular
        // remedy text never named `--clobber-create-once-seeds`. `create_once` comes from
        // [`crate::cli::commands::adopt::is_create_once_seed`] -- the identical predicate
        // `alef adopt` itself gates that flag on -- so this report and that refusal can
        // never drift apart again. ~keep
        if has_frozen_files {
            let (create_once_frozen, adoptable_frozen): (Vec<&FrozenFile>, Vec<&FrozenFile>) =
                frozen_generated_files.iter().partition(|frozen| frozen.create_once);
            if !adoptable_frozen.is_empty() {
                crate::bin_cli::output::line(
                    "Frozen generated files detected (alef owns these paths but the files carry no \
                 provenance marker, so alef refuses to write them -- review each file, then either \
                 add the marker shown and rerun `alef generate`, or delete the file so generation \
                 can write it cleanly):",
                );
                for frozen in &adoptable_frozen {
                    crate::bin_cli::output::line(format_args!("  {}", frozen.path));
                    if let Some(near_miss) = &frozen.near_miss {
                        crate::bin_cli::output::line(format_args!(
                            "    close but not recognized: {near_miss:?} (alef accepts \"generated by alef\" \
                         case-insensitively)"
                        ));
                    }
                    match &frozen.remedy {
                        Some(remedy) => crate::bin_cli::output::line(format_args!("    add marker: {remedy}")),
                        None => crate::bin_cli::output::line(
                            "    this format has no comment syntax to carry a marker, so alef proves ownership \
                         through the committed .alef-ownership.toml record instead -- run `alef adopt \
                         <path> --write` to record it there, or delete the file so the next `alef generate` \
                         writes and records it directly",
                        ),
                    }
                }
            }
            if !create_once_frozen.is_empty() {
                crate::bin_cli::output::line(
                    "Frozen create-once seeds detected (alef writes each of these paths only once and \
                 never revisits them, so the missing provenance marker is deliberate, not a bug -- the \
                 file on disk is presumed to be your grown copy of the placeholder alef seeded, exactly \
                 the case `alef adopt` refuses by default. Review each one, confirm it holds nothing you \
                 wrote -- a byte-identical file has nothing to lose either way -- then run `alef adopt \
                 <path> --write --clobber-create-once-seeds`):",
                );
                for frozen in &create_once_frozen {
                    crate::bin_cli::output::line(format_args!("  {}", frozen.path));
                }
            }
        }
        // Report-only, never auto-deleted: see `verify_orphans`'s module doc for the
        // asymmetry between a missed report (status quo) and a wrong deletion
        // (unrecoverable). Folded into the hard-fail exit code below anyway, same as
        // frozen files, so CI actually surfaces a dropped emit instead of staying green
        // forever -- which is the exact failure mode that let Java's visitor files sit
        // as invisible orphans across releases. ~keep
        if has_orphan_files {
            crate::bin_cli::output::line(
                "Orphaned generated files detected (alef's marker is present but the current run's \
             backends would not produce these paths -- a backend may have stopped emitting them, \
             they were dropped from generation config, or the file is a create-once seed alef only \
             writes when absent; review each and delete by hand if genuinely stale, alef never \
             deletes automatically):",
            );
            for path in &orphan_generated_files {
                crate::bin_cli::output::line(format_args!("  {path}"));
            }
        }
        // Not folded into missing/frozen: this is debt `collect_managed_surface`
        // hit while building the surface those two lists come from, not a
        // conclusion drawn *from* the surface. Naming it separately is what makes
        // a report that hit this debt distinguishable from one that genuinely
        // found nothing wrong -- a missing section here would look identical to
        // a clean run. ~keep
        if has_stage_failures {
            crate::bin_cli::output::line(
                "Generation debt detected while collecting the managed surface (missing/frozen \
             files above are still accurate; this is additional, separate debt):",
            );
            for failure in &stage_failures {
                crate::bin_cli::output::line(format_args!("  {failure}"));
            }
        }
    }
    super::super::verify_outcome::ensure_success(
        !stale.is_empty()
            || has_missing_files
            || has_missing_gitignored_files
            || has_adoptable_frozen_files
            || has_orphan_files
            || has_abi_disagreement
            || has_stage_failures,
        has_version_issues,
        snippet_coverage_issues.len(),
        report_only,
    )?;
    super::ensure_required_records_tracked(&untracked_records, report_only)?;
    ensure_configured_snippet_directories_exist(&missing_snippet_roots, report_only)?;
    Ok(None)
}

/// Dump, at debug level, the exact path set the orphan report is diffed against.
///
/// An orphan finding is a *difference* between two sets, and only one of them is printed: the
/// report names the files on disk and says nothing about the surface they were missing from.
/// That makes the two most common explanations indistinguishable from the output alone -- a
/// genuinely dropped emit, versus a managed surface that came back short because a stage failed
/// or because the run was language-filtered -- and it is why an orphan report and `alef generate`'s
/// own "unrecorded alef-marked file" warning can name different files without either being wrong:
/// they are diffs against different sets (`collect_managed_surface` over every configured
/// language here, versus this run's recorded output under one sweep root there, git-tracked-only
/// and manifest-gated).
///
/// Printing the surface is what makes that difference checkable instead of arguable: run
/// `alef verify -vv`, diff this list against the paths `alef generate` reports, and the gap names
/// itself. Debug level, not info: it is one line per managed file on a consumer tree. ~keep
fn log_managed_surface(managed_paths: &std::collections::HashSet<std::path::PathBuf>) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    let mut paths: Vec<&std::path::PathBuf> = managed_paths.iter().collect();
    paths.sort();
    tracing::debug!(
        "managed surface the orphan report is diffed against: {} path(s)",
        paths.len()
    );
    for path in paths {
        tracing::debug!("  managed: {}", path.display());
    }
}

/// The crate's configured `docs.snippets` roots that are not on disk, as
/// `<crate>: <configured entry> (resolved to <absolute path>)`.
///
/// `alef verify` had no opinion on this at all: it checks generated-file hashes and
/// generated-snippet coverage-ledger freshness, neither of which asks whether the roots those
/// snippets are configured to live in exist. A `dirs`/`inline_dirs` entry pointing at a path
/// that was renamed or never created is real config drift, and every snippet check that walks
/// it reports a clean run having examined nothing.
///
/// `alef all` already refuses the same condition (`docs::build_snippet_context`), but only as a
/// docs-stage error -- and `verify` reaches that same stage through
/// `find_missing_and_frozen_generated_files`, where a stage error is deliberately downgraded to
/// a debug log so an unrelated docs failure cannot fail an ownership question. So the condition
/// was already being detected during `verify` and then discarded. This asks the question
/// directly instead of trying to recover it from a swallowed stage error.
///
/// `exclude` is deliberately not applied, matching `build_snippet_context`: a root that is
/// excluded from discovery is still a root the configuration claims exists. ~keep
fn missing_snippet_directories(
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Vec<String> {
    let Some(snippets) = config.docs.as_ref().and_then(|docs| docs.snippets.as_ref()) else {
        return Vec::new();
    };
    snippets
        .dirs
        .iter()
        .chain(&snippets.inline_dirs)
        .filter_map(|dir| {
            let resolved = base_dir.join(dir);
            (!resolved.exists()).then(|| {
                format!(
                    "[{}] {} (resolved to {})",
                    config.name,
                    dir.display(),
                    resolved.display()
                )
            })
        })
        .collect()
}

/// Fail `alef verify` when a configured snippet root does not exist.
///
/// Kept out of [`super::super::verify_outcome::ensure_success`] for the same reason
/// [`super::ensure_required_records_tracked`] is: nothing here is stale and nothing regenerates
/// it, so "generated bindings, versions, or snippet coverage are out of date" would name the
/// wrong cause and the wrong fix. `report_only` short-circuits after the caller has already
/// printed the roots, matching how every other verify failure downgrades to a report. ~keep
fn ensure_configured_snippet_directories_exist(missing: &[String], report_only: bool) -> Result<()> {
    if report_only || missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "configured docs.snippets roots do not exist: {}. Fix the dirs/inline_dirs entries in \
         alef.toml or create the directories -- until then every snippet check that walks them \
         reports a clean run having examined nothing",
        missing.join(", ")
    )
}
