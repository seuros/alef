//! The "ordering" pre-flight gate for `docs.snippets.validation_level = "compile"`: whether a
//! snippet's session has a real mechanism to have produced its artifact, checked purely from
//! static configuration, before a single toolchain runs. Split out of `runner` under the repo's
//! file-size cap.

use super::{effective_validation_level, session_resolution};
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{Snippet, SnippetAnnotationKind, ValidationLevel};
use std::collections::{BTreeMap, HashMap};

/// Per-language counts of snippets whose effective validation level requires a compiled artifact
/// (`Compile`/`TypeCheck`/`Run`) but whose session cannot be relied on to have produced it: either
/// no configured session claims the snippet's language at all, the claim is ambiguous, or the
/// session that does claim it has no real `before` build step -- see `session_has_build_step`.
///
/// Computed purely from `snippets`, `sessions`, and `requested_level`: the exact inputs
/// `run_validation` is about to use, never a second walk of the filesystem or a second discovery
/// pass. That is the same guarantee `gap_coverage` gives `VerifyCoverage`/`GapCoverage` -- a count
/// that describes what this run is actually about to do, not an estimate computed some other way.
///
/// `alef docs`/`alef all` never run a per-language build on their own behalf (see
/// `bin_cli::all_commands::warn_if_snippet_validation_needs_build`'s doc for why), so a session
/// with nothing configured to run first has no mechanism to have produced the artifact its
/// snippets are about to validate against. That is the literal shape of "no stage in the invoked
/// pipeline produced this artifact" -- the ordering half of alef #256. ~keep
pub(crate) fn missing_build_dependency(
    snippets: &[Snippet],
    sessions: &HashMap<String, SessionSpec>,
    requested_level: ValidationLevel,
) -> BTreeMap<crate::snippets::types::Language, usize> {
    let mut counts = BTreeMap::new();
    for snippet in snippets {
        if let Some(annotation) = &snippet.annotation
            && annotation.kind == SnippetAnnotationKind::Skip
        {
            continue;
        }
        if effective_validation_level(snippet, requested_level) < ValidationLevel::Compile {
            continue;
        }
        let guaranteed = match session_resolution::resolve_session_claim(snippet, sessions, |spec| spec.language) {
            session_resolution::SessionClaim::Claimed(key) => sessions.get(key).is_some_and(session_has_build_step),
            session_resolution::SessionClaim::Unclaimed | session_resolution::SessionClaim::Ambiguous(_) => false,
        };
        if !guaranteed {
            *counts.entry(snippet.language).or_insert(0_usize) += 1;
        }
    }
    counts
}

/// Shell idioms that always exit 0 and do nothing, so a `before` command that is, once trimmed,
/// exactly one of these can never have built anything -- see `session_has_build_step`.
const NO_OP_BEFORE_COMMANDS: &[&str] = &["true", ":"];

/// Whether `spec` has at least one `before` command that is not a no-op shell idiom.
///
/// This is a narrower question than "did this session build its artifact" -- `before` has not run
/// yet at the point `missing_build_dependency` calls this (it is a pre-flight check over static
/// config, ahead of `run_validation`/`prepare_sessions_isolated`), so no exit code or filesystem
/// evidence exists yet to inspect. `!spec.before.is_empty()` used to stand in for "guaranteed",
/// but `before = ["true"]` is non-empty and guarantees nothing: the command always succeeds
/// without doing any work. This closes that specific, demonstrated bypass by rejecting the known
/// no-op idioms a `before` list can be made of; it does not -- and cannot, from config alone --
/// prove any other configured command actually produces the artifact its snippets need. ~keep
fn session_has_build_step(spec: &SessionSpec) -> bool {
    spec.before
        .iter()
        .any(|command| !NO_OP_BEFORE_COMMANDS.contains(&command.trim()))
}

#[cfg(test)]
mod build_dependency_tests;
