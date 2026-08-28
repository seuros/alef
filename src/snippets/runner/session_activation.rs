//! Decides which configured sessions `run_validation` can skip activating (running the `before`
//! hook for) because every snippet that would use them is already a `--changed-only` cache hit.
//!
//! Split out of `runner.rs` under the repo's file-size cap; the logic itself is `run_validation`'s
//! -- see its call site for the reordering this exists to support.

use super::{RunnerConfig, cached_result, session_resolution};
use crate::snippets::session::{SessionSpec, ValidationSession};
use crate::snippets::types::Snippet;
use std::collections::HashMap;

/// Every snippet that unambiguously claims a configured session, keyed by that session's target
/// name -- the same resolution `session_for`/`session_preparation_error` use, restricted to
/// `SessionClaim::Claimed` because that is the only outcome [`needs_before_hook`]'s predicate can
/// act on safely (see its own doc). Built once per run and reused for every target, rather than
/// re-resolving every snippet inside the predicate per session: `resolve_session_claim` walks
/// every candidate for a language, so doing that per (snippet, session) pair would turn an
/// O(snippets) resolution into O(snippets * sessions). ~keep
///
/// Resolved against the *full* `all_sessions` map, matching `sessions_needed_for_preparation` and
/// `session_prep::session_preparation_error` -- see their doc comments for why claim resolution
/// must always see every configured session, not just the subset a given run prepares. Only
/// `SessionClaim::Claimed` snippets are recorded: an `Ambiguous` claim resolves to no session in
/// `session_for` regardless, so a target that only ever appears as an ambiguous candidate is left
/// with an empty claim list here and [`needs_before_hook`] conservatively keeps activating it,
/// unchanged from before this module existed. ~keep
pub(super) fn claimed_snippets_by_target<'snippet, 'session>(
    snippets: &'snippet [Snippet],
    all_sessions: &'session HashMap<String, SessionSpec>,
) -> HashMap<&'session str, Vec<&'snippet Snippet>> {
    let mut claims: HashMap<&'session str, Vec<&'snippet Snippet>> = HashMap::new();
    for snippet in snippets {
        if let session_resolution::SessionClaim::Claimed(key) =
            session_resolution::resolve_session_claim(snippet, all_sessions, |spec| spec.language)
        {
            claims.entry(key).or_default().push(snippet);
        }
    }
    claims
}

/// The activation-skip predicate `prepare_sessions_isolated_with_activation_filter` runs per
/// session, once its fingerprint is known: a session whose every claiming snippet already has a
/// `--changed-only` cache hit has nothing for its `before` hook to build for, so activation can
/// skip running it. `cached_result` is the exact same predicate `validate_one`/`batch_level` use
/// moments later for the same snippet against the same (now-fingerprinted) session, so "cached
/// here" and "cached there" cannot drift apart -- the fingerprint is fixed by phase one of session
/// preparation before this predicate ever runs. A target with no recorded claim (unclaimed by any
/// snippet, or kept only to protect a sibling's scratch during the purge) keeps the previous
/// eager-activation behaviour: `true` is the safe default here, never `false`. ~keep
pub(super) fn needs_before_hook<'claims>(
    claims_by_target: &'claims HashMap<&str, Vec<&Snippet>>,
    config: &'claims RunnerConfig,
) -> impl Fn(&str, &ValidationSession) -> bool + Sync + 'claims {
    move |target: &str, session: &ValidationSession| -> bool {
        match claims_by_target.get(target) {
            Some(claimed) if !claimed.is_empty() => {
                !claimed
                    .iter()
                    .copied()
                    .all(|snippet| cached_result(snippet, config, Some(session)).is_some())
            }
            _ => true,
        }
    }
}
