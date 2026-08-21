//! Classifies a snippet whose *session* never became usable -- as opposed to one whose validator
//! ran and produced a real result. Before this module existed, both `validate_one` (the
//! fail-fast path) and `batch::group_batchable_snippets` (the batched/parallel path) built their
//! own `SnippetStatus::Error` result straight from the stringified session preparation error,
//! independently of each other. That duplication is exactly how alef defect #142 happened: a
//! session's `before` hook (which builds this language's artifacts) timing out and a session's
//! manifest simply not existing produced the identical `SnippetStatus::Error`, with a message
//! that -- for the timeout case -- was just the raw "command timed out after 120s" text. A reader
//! could not tell an unbuilt artifact from a broken snippet from a misconfigured session; all
//! three read as "validation failed". One function, used by both dispatch paths, is what keeps
//! them from drifting apart again.

use super::{RunnerConfig, result};
use crate::snippets::session::SessionPreparationError;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationResult};
use std::collections::HashMap;

/// The preparation error (if any) that applies to `snippet`, looked up first by its explicit
/// `target` metadata and falling back to its language's own session.
pub(super) fn session_preparation_error<'a>(
    snippet: &Snippet,
    sessions: &HashMap<String, crate::snippets::session::ValidationSession>,
    errors: &'a HashMap<String, SessionPreparationError>,
) -> Option<&'a SessionPreparationError> {
    let target = snippet
        .metadata
        .target
        .as_ref()
        .map(|target| Language::normalize_session_target(target));
    if let Some(target) = target.as_deref() {
        if let Some(error) = errors.get(target) {
            return Some(error);
        }
        if sessions.contains_key(target) {
            return None;
        }
    }
    errors.get(&snippet.language.to_string())
}

/// Builds the `ValidationResult` for a snippet whose session never became usable.
///
/// An `ordering` preparation error -- the session's `before` hook hit `timeout_secs` while
/// building this language's artifacts, not while validating any particular snippet -- reclassifies
/// through the same `unresolved_dependency` bucket a validator's own dependency-shaped `Fail`
/// uses (see `runner::finalize_result`): both share the same root cause, an artifact `alef build`
/// was supposed to produce that validation could not see yet. Every other preparation failure (a
/// missing manifest, a missing working directory, a `before` hook that ran to completion and
/// failed on its own terms) stays `SnippetStatus::Error`: that is broken configuration, not a
/// build-order gap, and must not be waved through the way an ordering problem is.
pub(super) fn session_preparation_result(
    snippet: &Snippet,
    config: &RunnerConfig,
    error: &SessionPreparationError,
) -> ValidationResult {
    let status = if error.ordering {
        SnippetStatus::Unavailable
    } else {
        SnippetStatus::Error
    };
    let mut outcome = result(
        snippet,
        status,
        config.level,
        config.level,
        Some(error.message.clone()),
        0,
    );
    outcome.unresolved_dependency = error.ordering;
    outcome
}
