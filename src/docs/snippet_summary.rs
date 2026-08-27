//! Snippet-run summary reporting: severity choice and per-status attribution detail.
//!
//! Split out of `docs::mod` when that file crossed the 1,000-line cap. The concept boundary is
//! "turn a finished `RunSummary` into what the operator is told" -- severity selection plus the
//! `attribute_*` family that renders each status's detail line. Nothing here generates docs. ~keep

use std::collections::BTreeMap;

/// Whether every single result in `summary` is a graceful `unresolved_dependency` downgrade --
/// the shape `finalize_result` (in `snippets::runner`) produces when a real toolchain ran to
/// completion and reported a missing import/link/build target, never a defect in the generated
/// bindings. `alef docs`/`alef all` never build a language's real artifacts in the same
/// invocation (see the doc comment on the call site in `bin_cli::all_commands::handle`), so a
/// `docs.snippets.validation_level` of `compile`/`typecheck`/`run` reliably produces exactly this
/// shape until a separate `alef build` (or an explicit `alef snippets check --level compile`
/// after one) runs -- a correct configuration for its purpose, not a defect (task #542). `true`
/// only when nothing else contributed: not one real failure, not one plain toolchain-missing gap,
/// not one capability/declared cap. Used by both loud-reporting sites below to choose `info` over
/// `warn` for precisely this expected shape, while any admixture of a genuine problem keeps the
/// existing `warn` treatment. ~keep
pub(crate) fn corpus_is_entirely_unresolved_dependency(summary: &crate::snippets::types::RunSummary) -> bool {
    summary.total > 0 && summary.total == summary.unresolved_dependency
}

/// Decides whether a completed snippet validation run fails the crate, and how. Order matters:
/// hard failures (`Fail`/`Error`, which includes session/preparation errors — see
/// `session::prepare_sessions_isolated`) bail before a strict downgraded bail. A run can carry
/// both hundreds of downgrades and hundreds of real failures at once, and the strict downgraded
/// check used to run — and bail — before the failure check further down was ever reached, so a
/// consumer investigating "N downgraded" never learned the run had failed outright. A user must
/// never be told "downgraded" when something actually failed. Factored out of `validate_snippets`
/// so this ordering is directly testable against a constructed `RunSummary`, without a real
/// toolchain or filesystem — see `docs::tests::strict_bail_order`. ~keep
pub(crate) fn enforce_snippet_summary(
    crate_name: &str,
    strict: bool,
    summary: &crate::snippets::types::RunSummary,
) -> anyhow::Result<()> {
    // Reported first, and unconditionally, so it cannot be missed regardless of what the
    // strict-gated checks below decide (task #488): not one result in this run reached its
    // requested level. This does NOT bail outright, unlike `run_check`'s equivalent
    // `RunSummary::checked_nothing` gate (`cli::commands::snippets`) -- `alef docs`/`alef all`
    // structurally cannot guarantee a fresh build ran in the same invocation (see the
    // `unresolved_dependency` comment just below, task #186), so a corpus that is entirely
    // `unresolved_dependency`-unavailable is an expected shape for THIS pipeline, not a defect.
    // Loud reporting, not a hard failure, is the correct answer for the same reason task #186's
    // exemptions below are not bails either.
    //
    // Severity is `info`, not `warn`, when that expected shape is the *whole* story (task #542):
    // a consumer running `alef snippets check --level compile` after a real `alef build` gets a
    // real check, so a plain `alef all`/`alef docs` reporting the same, structurally-unsatisfiable
    // precondition as a warning on every single run trained operators to distrust it for a
    // configuration that is correct for its purpose. Any admixture of a real failure or a genuine
    // toolchain gap still warns, because those are never expected. ~keep
    if summary.checked_nothing() {
        if corpus_is_entirely_unresolved_dependency(summary) {
            tracing::info!(
                total = summary.total,
                "docs.snippets for crate `{}` validated {} snippet(s) and NOT ONE reached the requested level -- \
                 every result is an unresolved dependency on a missing build artifact, the expected shape when \
                 `alef all`/`alef docs` validate without a preceding `alef build`; run `alef build` then `alef \
                 snippets check --level compile` (or the configured level) to validate for real",
                crate_name,
                summary.total
            );
        } else {
            tracing::warn!(
                total = summary.total,
                "docs.snippets for crate `{}` validated {} snippet(s) and NOT ONE reached the requested level -- \
                 every result was a failure, a skip, an unavailable environment gap, or capped below what was \
                 requested; the level this run claims to check was not actually checked anywhere in this corpus",
                crate_name,
                summary.total
            );
        }
    }
    // `unresolved_dependency` is never a defect in the generated bindings: it is set only when a
    // real toolchain ran to completion and reported a missing import/link/build target (see
    // `finalize_result`'s doc comment in `snippets::runner`), and every caller of this function
    // (`alef docs`, `alef all`) never runs a full per-language build in the same invocation --
    // see the doc comment on this function's call site in `bin_cli::all_commands::handle`. Strict
    // mode failing on it indistinguishably from a genuine content defect is what trained
    // operators to distrust an `alef all` failure (task #186): codegen completed and wrote
    // correct output, yet the run reported failure for a precondition `alef all`/`alef docs`
    // structurally cannot satisfy on their own. A toolchain that is simply missing from `PATH`
    // (`toolchain_missing`) is still a real environment gap unrelated to any build artifact, and
    // still fails strict mode. ~keep
    let toolchain_missing = summary.unavailable - summary.unresolved_dependency;
    if toolchain_missing > 0 && strict {
        anyhow::bail!(
            "strict snippet validation failed for crate `{}`: {} unavailable due to a missing toolchain \
             (unrelated to a missing build artifact){}",
            crate_name,
            toolchain_missing,
            attribute_unavailable(summary)
        );
    }
    if summary.capability_capped > 0 {
        tracing::warn!(
            capped = summary.capability_capped,
            "docs.snippets validated {} snippet(s) below the requested level because their validator caps lower; \
             these pass strict mode, because the level is unreachable for that language rather than degraded{}",
            summary.capability_capped,
            attribute_capability_capped(summary)
        );
    }
    // `declared_capped` results pass exactly like `capability_capped` ones do, and for the same
    // structural reason this function already warns about the latter: a bare "N passed" leaves a
    // consumer with no way to learn that `docs.snippets.validation_level` was not actually applied
    // to every snippet. The gap this closes is concrete, not hypothetical: `alef e2e generate`
    // stamps every fixture snippet's front matter with `level: typecheck` (see
    // `e2e::snippets::render_snippet_markdown`), which silently caps `validation_level = "run"`
    // down to typecheck for all of them — previously reported as an unqualified `Pass` with no
    // trace anywhere in this pipeline's output. ~keep
    if summary.declared_capped > 0 {
        tracing::warn!(
            declared_capped = summary.declared_capped,
            "docs.snippets validated {} snippet(s) below the requested level because their own front-matter \
             `level:` declares a lower ceiling; these pass strict mode as a satisfied per-snippet contract, \
             but the requested level was not actually applied to them{}",
            summary.declared_capped,
            attribute_declared_capped(summary)
        );
    }
    if summary.unavailable > 0 {
        let toolchain_missing = summary.unavailable - summary.unresolved_dependency;
        // `toolchain_missing == 0` here means every `unavailable` result in this bucket is
        // `unresolved_dependency` -- the same expected-build-gap shape `checked_nothing` above
        // reports at `info`, just scoped to this one bucket rather than the whole corpus (a
        // multi-language run where some sessions already have their artifacts built and others
        // don't must not have its legitimate half masked by, nor mask, its expected half). ~keep
        if toolchain_missing == 0 {
            tracing::info!(
                unavailable = summary.unavailable,
                unresolved_dependency = summary.unresolved_dependency,
                "docs.snippets skipped {} snippet validation(s) because `alef all`/`alef docs` does not build \
                 first -- every one is an unresolved dependency on a missing build artifact, not a real \
                 toolchain gap{}",
                summary.unavailable,
                attribute_unavailable(summary)
            );
        } else {
            tracing::warn!(
                unavailable = summary.unavailable,
                unresolved_dependency = summary.unresolved_dependency,
                toolchain_missing,
                "docs.snippets skipped {} snippet validation(s) because required toolchains were unavailable ({} \
                 unresolved dependency, {} toolchain missing){}",
                summary.unavailable,
                summary.unresolved_dependency,
                toolchain_missing,
                attribute_unavailable(summary)
            );
        }
    }
    if summary.has_failures() {
        anyhow::bail!(
            "snippet validation failed for crate `{}`: {} failed, {} errors{}{}",
            crate_name,
            summary.failed,
            summary.errors,
            attribute_results(summary, crate::snippets::types::SnippetStatus::Fail),
            attribute_results(summary, crate::snippets::types::SnippetStatus::Error)
        );
    }
    if summary.downgraded > 0 && strict {
        anyhow::bail!(
            "strict snippet validation failed for crate `{}`: {} validation(s) downgraded{}",
            crate_name,
            summary.downgraded,
            attribute_results(summary, crate::snippets::types::SnippetStatus::Downgraded)
        );
    }
    Ok(())
}

/// Human label for why a result's effective level differs from what was requested. Kept
/// alongside the attribution formatting it feeds — `DowngradeReason` itself stays a plain data
/// enum with no presentation concerns. ~keep
fn downgrade_reason_label(reason: crate::snippets::types::DowngradeReason) -> &'static str {
    use crate::snippets::types::DowngradeReason;
    match reason {
        DowngradeReason::Declared => "author declared this level via front matter",
        DowngradeReason::Annotation => "author suppressed via annotation",
        DowngradeReason::ValidatorCapability => "validator cannot reach this level",
        DowngradeReason::Environment => "environment could not reach this level",
    }
}

/// Name the snippets behind a strict-mode count so the failure is actionable.
///
/// A bare total ("261 validation(s) downgraded") gives a consumer no entry point: the achieved
/// level is not recorded in the emitted snippet frontmatter, so there is no other way to learn
/// which snippets regressed or from what level. Groups by language and bounds the per-language
/// sample, so a large run stays readable while still naming concrete ids to start from. Within a
/// language, also breaks the count down by `downgrade_reason` — "author declared this level" and
/// "environment could not reach this level" call for entirely different fixes, and collapsing
/// them into one count told a consumer nothing about which one applies. ~keep
pub(crate) fn attribute_results(
    summary: &crate::snippets::types::RunSummary,
    status: crate::snippets::types::SnippetStatus,
) -> String {
    attribute(summary, |result| result.status == status)
}

/// Same attribution, for the `Pass` results a validator's declared or structural ceiling capped
/// below the requested level. These never fail strict, but a consumer watching the summary count
/// climb still needs to know which snippets and why. ~keep
pub(crate) fn attribute_capability_capped(summary: &crate::snippets::types::RunSummary) -> String {
    attribute(summary, |result| result.capability_capped)
}

/// Same attribution, for the `Pass` results a snippet's own front-matter `level:` contract capped
/// below the requested level (`DowngradeReason::Declared`). These never fail strict either, but
/// unlike `capability_capped` this ceiling came from the snippet's content, not its language — a
/// consumer needs to know that too, especially when the "author" is a code generator that stamped
/// the same declared level onto every snippet it emitted rather than a person choosing it. ~keep
pub(crate) fn attribute_declared_capped(summary: &crate::snippets::types::RunSummary) -> String {
    attribute(summary, |result| {
        result.downgrade_reason == Some(crate::snippets::types::DowngradeReason::Declared)
    })
}

/// Per-language counts for `Unavailable` results, split by cause. Deliberately not
/// `attribute_results`: the remediation for `unresolved_dependency` is "run `alef build`" and for
/// a plain toolchain gap is "install the toolchain", and both apply to the whole language batch,
/// not to one snippet — three sample ids and a `+N more` told a consumer nothing a count didn't
/// already, since the fix is the same for every snippet in the batch. This also sidesteps two bugs
/// `attribute_results` had for this status: its `[reasons]` bracket reads `downgrade_reason`,
/// which is `None` for every `Unavailable` result by construction (see the `debug_assert!` in
/// `runner::finalize_result`), so the bracket was always empty; and its `(requested -> effective)`
/// arrow implied a level downgrade that never happened here — `Unavailable` results carry no
/// downgrade at all. ~keep
pub(crate) fn attribute_unavailable(summary: &crate::snippets::types::RunSummary) -> String {
    #[derive(Default)]
    struct LanguageCounts {
        unresolved_dependency: usize,
        toolchain_missing: usize,
    }

    let mut by_language: BTreeMap<String, LanguageCounts> = BTreeMap::new();
    for result in summary
        .results
        .iter()
        .filter(|result| result.status == crate::snippets::types::SnippetStatus::Unavailable)
    {
        let entry = by_language.entry(result.snippet.language.to_string()).or_default();
        if result.unresolved_dependency {
            entry.unresolved_dependency += 1;
        } else {
            entry.toolchain_missing += 1;
        }
    }
    if by_language.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (language, counts) in by_language {
        out.push_str(&format!(
            "\n  {language}: {} unresolved dependency, {} toolchain missing",
            counts.unresolved_dependency, counts.toolchain_missing
        ));
    }
    out
}

fn attribute(
    summary: &crate::snippets::types::RunSummary,
    matches: impl Fn(&crate::snippets::types::ValidationResult) -> bool,
) -> String {
    const SAMPLE_PER_LANGUAGE: usize = 3;

    #[derive(Default)]
    struct LanguageGroup {
        count: usize,
        reasons: BTreeMap<&'static str, usize>,
        sample: Vec<String>,
    }

    let mut by_language: BTreeMap<String, LanguageGroup> = BTreeMap::new();
    for result in summary.results.iter().filter(|result| matches(result)) {
        let entry = by_language.entry(result.snippet.language.to_string()).or_default();
        entry.count += 1;
        if let Some(reason) = result.downgrade_reason {
            *entry.reasons.entry(downgrade_reason_label(reason)).or_default() += 1;
        }
        if entry.sample.len() < SAMPLE_PER_LANGUAGE {
            let id = result.snippet.id.clone().unwrap_or_else(|| {
                format!(
                    "{}:{}",
                    result.snippet.source_origin.path.display(),
                    result.snippet.source_origin.line
                )
            });
            entry.sample.push(format!(
                "{id} ({} -> {})",
                result.requested_level, result.effective_level
            ));
        }
    }
    if by_language.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (language, group) in by_language {
        let remainder = group.count.saturating_sub(group.sample.len());
        let suffix = if remainder > 0 {
            format!(", +{remainder} more")
        } else {
            String::new()
        };
        let reasons = group
            .reasons
            .iter()
            .map(|(label, count)| format!("{label}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let reason_suffix = if reasons.is_empty() {
            String::new()
        } else {
            format!(" [{reasons}]")
        };
        out.push_str(&format!(
            "\n  {language}: {}{reason_suffix} -- {}{suffix}",
            group.count,
            group.sample.join(", ")
        ));
    }
    out
}
