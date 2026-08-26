//! `alef adopt`'s command handler: one managed surface per invocation, one batch of targets
//! against it, one bounded report.
//!
//! Split out of `aux_commands` because the three responsibilities below are what makes the
//! recovery path affordable, and they are worth reading together. `alef adopt` is the way out
//! of the ownership-ledger dead end -- a consumer whose gitignored ownership manifest was lost
//! has no durable proof alef ever wrote its managed files, so alef refuses to regenerate them,
//! and `alef adopt --converged-only --write` re-records ownership for the files whose bytes
//! already match generated output. A recovery command has to be cheap enough to actually run.
//!
//! 1. [`managed_surface`] renders the surface **once for the whole invocation**, per crate,
//!    above the target loop. It is a parameter of the batch below, not something the batch can
//!    reach, so no future edit can reintroduce a per-target regeneration.
//! 2. `adopt::run_batch` resolves every target against that one surface, sharing classification,
//!    diff rendering and the ownership-record write across targets.
//! 3. [`report`] merges the per-target reports and bounds what is printed, naming an exact
//!    count for anything it withholds.

use anyhow::Result;
use std::path::Path;

use crate::cli::{commands, dispatch, pipeline};

use super::dispatch::DispatchContext;
use super::helpers::{StageFailure, collect_managed_surface, load_config, resolve_languages};

pub(crate) mod report;

/// Everything one crate's configuration would cause alef to emit, plus the stage failures
/// that render tolerated.
type CrateSurface = (Vec<crate::core::backend::GeneratedFile>, Vec<StageFailure>);

/// Render the managed surface for every selected crate, exactly once per crate.
///
/// The diff a human consents to has to be against the bytes a real generate would write, so
/// the full managed surface -- every stage `alef all` writes, not a hand-maintained subset of
/// it -- is what feeds it. Shared verbatim with `alef verify`'s frozen-file report so the
/// report and the remedy for the same fact cannot disagree; see `collect_managed_surface`. A
/// stage failure it tolerated is only ours to ignore when none of the requested `targets`
/// could have come from that stage -- otherwise this run cannot answer the ownership question
/// the operator actually asked, and must say so rather than adopt against possibly-stale
/// bytes. ~keep
///
/// `render_crate` is a parameter rather than an inlined call so the "once per crate, never per
/// target" property is *counted* by a test instead of inferred from how the loop happens to be
/// nested today. The generic crate type keeps that test from having to build a
/// `ResolvedCrateConfig`. ~keep
pub(crate) fn managed_surface<C, R>(
    crates: &[C],
    targets: &[String],
    base_dir: &Path,
    mut render_crate: R,
) -> Result<Vec<commands::adopt::ManagedOutput>>
where
    R: FnMut(&C) -> Result<CrateSurface>,
{
    let mut managed = Vec::new();
    for crate_config in crates {
        let (surface, stage_failures) = render_crate(crate_config)?;
        for failure in &stage_failures {
            if failure.affects_any(targets) {
                anyhow::bail!(
                    "[{}] {} -- this affects one of the requested targets, so `alef adopt` \
                     cannot answer for it",
                    failure.stage,
                    failure.message
                );
            }
            tracing::debug!(
                stage = failure.stage,
                "tolerating stage failure: no requested target comes from this stage: {}",
                failure.message
            );
        }
        managed.extend(commands::adopt::managed_outputs(&surface, base_dir));
    }
    Ok(managed)
}

pub(crate) fn handle(
    targets: Vec<String>,
    write: bool,
    converged_only: bool,
    clobber_create_once_seeds: bool,
    context: &DispatchContext,
) -> Result<()> {
    let config_path = &context.config_path;
    let base_dir = std::env::current_dir()?;
    let (_workspace, resolved) = load_config(config_path)?;
    let crates_to_process = dispatch::select_crates(&resolved, &context.crate_filter)?;

    let managed = managed_surface(&crates_to_process, &targets, &base_dir, |resolved_cfg| {
        let languages = resolve_languages(resolved_cfg, None)?;
        let api = pipeline::extract(resolved_cfg, config_path, false)?;
        collect_managed_surface(&languages, &api, resolved_cfg, config_path, &base_dir)
    })?;

    let options = commands::adopt::AdoptBatchOptions {
        base_dir: base_dir.clone(),
        write,
        converged_only,
        clobber_create_once_seeds,
    };
    let outcome = commands::adopt::run_batch(&targets, &options, &managed)?;

    let summary = report::AdoptSummary::merge(&outcome, !write);
    report::emit(&report::render(&summary, converged_only));
    report::log_diagnostics(&summary);

    report_failures(&outcome, targets.len())
}

/// One target's refusal must not silently cancel the other fifty-three, and the exit status
/// has to name how many failed rather than only the first. See `adopt::batch::run_batch`. ~keep
fn report_failures(outcome: &commands::adopt::AdoptBatchOutcome, requested: usize) -> Result<()> {
    let failures: Vec<(&str, &anyhow::Error)> = outcome.failures().collect();
    if failures.is_empty() {
        return Ok(());
    }
    for (target, error) in &failures {
        tracing::error!("{target}: {error:#}");
    }
    anyhow::bail!(
        "{} of {} adopt target(s) could not be adopted; each is reported above",
        failures.len(),
        requested
    )
}

#[cfg(test)]
mod tests;
