//! What an `alef snippets` gap check actually compared -- stated in the report, every run.
//!
//! Every finding the gap detector produces is a *negative* claim (nothing unreferenced, no
//! missing language variant, no dangling include target), and a report made only of negative
//! claims is indistinguishable from a report that compared nothing. A consumer whose
//! `alef.toml` omitted `required_languages`, `docs_dirs` and `include_base_paths` read
//! "No gaps found." for a run in which the language-parity check never executed and not one
//! documentation page was opened -- the command answered "nothing is missing" when it meant
//! "I was given nothing to compare against".
//!
//! This module states the scope in numbers alongside the verdict, and names each unset input
//! together with the check class its absence disables. It is the same fix
//! `alef snippets audit` needed for a bare "Audit clean: no issues found." over a docs tree it
//! never opened, and the same one [`crate::bin_cli::verify_coverage`] made for `alef verify`:
//! the floor is not "check more", it is "never let the report read as a bigger claim than the
//! check made". ~keep

use crate::snippets::types::Language;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The measured scope of one gap check.
///
/// Built by [`crate::snippets::gaps::detect_gaps`] from the walks it already performed, never
/// from a second walk of its own: a coverage report derived independently of the checks it
/// describes can disagree with them, which would make it worse than no report at all. ~keep
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapCoverage {
    /// Configured snippet roots.
    pub snippet_roots: usize,
    /// Snippet files discovered under those roots, after exclusions.
    pub snippets_discovered: usize,
    /// Configured documentation roots.
    pub docs_roots: usize,
    /// Documentation pages actually opened and parsed for include directives. Zero here means
    /// the include-target check examined nothing, whatever the root count says.
    pub docs_pages_scanned: usize,
    /// References found by reading documentation pages (`--8<--` includes, MDX content
    /// imports).
    pub include_references: usize,
    /// References supplied by configuration rather than discovered: coverage ledgers,
    /// `[crates.readme]` snippet mappings, Astro content collections. These make a snippet
    /// count as referenced without any documentation page mentioning it, which is exactly how
    /// an unconfigured run reports zero orphans. ~keep
    pub configured_references: usize,
    /// Languages every snippet group is required to provide.
    pub required_languages: usize,
    /// Snippet groups the language-parity check compared. Zero means the check produced no
    /// finding because it had nothing to iterate, not because parity holds.
    pub language_groups: usize,
    /// `pymdownx.snippets`-style base paths used to resolve include targets.
    pub include_base_paths: usize,
}

impl GapCoverage {
    /// The coverage report, one line per element, ready for `output::line`.
    ///
    /// A pure function returning lines rather than printing them, so the numbers and the
    /// wording are unit-testable: the commands write through `output::line` straight to
    /// stdout, which an in-process test cannot intercept. ~keep
    #[must_use]
    pub fn report_lines(&self) -> Vec<String> {
        vec![
            "Gap coverage (what this run compared, so a clean result is not read as more than it is):".to_owned(),
            format!(
                "  snippet roots: {} configured, {} snippet file(s) discovered",
                self.snippet_roots, self.snippets_discovered
            ),
            format!(
                "  documentation roots: {} configured, {} page(s) opened and parsed{}",
                self.docs_roots,
                self.docs_pages_scanned,
                if self.docs_pages_scanned == 0 {
                    " -- NO documentation page entered this result"
                } else {
                    ""
                }
            ),
            format!(
                "  references: {} discovered in documentation ({} include base path(s)), {} supplied by \
                 configuration (coverage ledgers, [crates.readme] snippets, Astro collections)",
                self.include_references, self.include_base_paths, self.configured_references
            ),
            format!(
                "  language parity: {} required language(s) across {} snippet group(s){}",
                self.required_languages,
                self.language_groups,
                if self.required_languages == 0 || self.language_groups == 0 {
                    " -- the missing-language-variant check produced no finding because it compared nothing"
                } else {
                    ""
                }
            ),
        ]
    }
}

/// One gap-check input that was left unset, and the check class its absence disables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsetGapInput {
    /// The `alef.toml` key under `[crates.docs.snippets]`.
    pub key: &'static str,
    /// The equivalent `alef snippets gaps` flag.
    pub flag: &'static str,
    /// What the run did NOT check as a result.
    pub consequence: &'static str,
    /// Whether emptiness makes the *verdict* vacuous -- the check silently produced no finding
    /// because it had nothing to iterate.
    ///
    /// `include_base_paths` is deliberately not vacuous: an unset base-path list makes include
    /// targets resolve against the docs root only, which turns an unresolvable target into a
    /// *reported* missing reference and its snippet into a *reported* orphan. That
    /// over-reports; it cannot manufacture a false clean, so gating a CI run on it would fail
    /// every project that legitimately has no `pymdownx.snippets` `base_path`. ~keep
    pub vacuous: bool,
}

/// Name every gap-check input left unset by this invocation.
///
/// Takes the raw pre-fallback inputs, because unset-ness is only observable before the
/// caller's defaulting runs: `alef snippets gaps` substitutes the docs roots for an empty
/// `--include-base-path` list, after which the two cases are indistinguishable. ~keep
#[must_use]
pub fn unset_gap_inputs(
    docs_dirs: &[PathBuf],
    required_languages: &[Language],
    include_base_paths: &[PathBuf],
) -> Vec<UnsetGapInput> {
    let mut unset = Vec::new();
    if docs_dirs.is_empty() {
        unset.push(UnsetGapInput {
            key: "docs_dirs",
            flag: "--docs",
            consequence: "no documentation page was opened, so the missing-include-target check did NOT run and \
                          every snippet's referenced/orphaned status came only from configured references",
            vacuous: true,
        });
    }
    if required_languages.is_empty() {
        unset.push(UnsetGapInput {
            key: "required_languages",
            flag: "-L/--required-languages",
            consequence: "the missing-language-variant check did NOT run, so no language parity was compared",
            vacuous: true,
        });
    }
    if include_base_paths.is_empty() {
        unset.push(UnsetGapInput {
            key: "include_base_paths",
            flag: "--include-base-path",
            consequence: "`--8<--` targets resolve against the documentation root only, so includes written \
                          against a pymdownx.snippets base_path resolve to the wrong candidate path",
            vacuous: false,
        });
    }
    unset
}

/// Whether any unset input makes the verdict vacuous, and therefore must fail a strict run.
#[must_use]
pub fn has_vacuous_input(unset: &[UnsetGapInput]) -> bool {
    unset.iter().any(|input| input.vacuous)
}

/// The prominent warning block for a partially configured gap check.
///
/// Returns no lines when every input is set, so a fully configured run stays quiet. ~keep
#[must_use]
pub fn unset_input_lines(unset: &[UnsetGapInput], strict: bool) -> Vec<String> {
    if unset.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "Gap check NOT fully configured: {} input(s) unset, each disabling a check class:",
        unset.len()
    )];
    for input in unset {
        lines.push(format!("  {} unset ({}): {}", input.key, input.flag, input.consequence));
    }
    if has_vacuous_input(unset) && !strict {
        lines.push(
            "  A clean result below therefore proves less than it appears to. Configure the keys above, or \
             pass --strict to make an unconfigured gap check fail instead of pass."
                .to_owned(),
        );
    }
    lines
}

#[cfg(test)]
mod tests;
