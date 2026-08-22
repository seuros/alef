//! Resolves which configured validation session -- if any -- claims a snippet, as a single
//! function every dispatch path and `session_preparation_error` share.
//!
//! ## Alef defect #127
//!
//! A hand-written snippet carries no `metadata.target` (that field is only ever populated from
//! front matter or from a generated snippet's coverage ledger), so before this module existed the
//! fallback was a literal string lookup: `sessions.get(&snippet.language.to_string())`, i.e. "is
//! there a configured session target spelled exactly like the bare language name". That only ever
//! matched by naming coincidence. A real consumer's `alef.toml` configures both
//! `[docs.snippets.sessions.typescript]` (its Node bindings) and `[docs.snippets.sessions.wasm]`
//! (its WASM bindings) -- both targeting `Language::TypeScript`. Every hand-written TypeScript
//! snippet silently landed on the session literally named `typescript` (an accident of naming,
//! not a deliberate "generic TypeScript fallback"), validated against the Node toolchain even
//! when it demonstrated WASM-only usage, while the `wasm` session never received a single
//! hand-written snippet and that gap was invisible in any report. The three other consumer repos
//! name the same two sessions `node` and `wasm` -- neither spells the bare language -- so their
//! hand-written TypeScript snippets got no session at all and validated in an isolated scratch
//! directory with no access to the real package, silently producing results that reflect nothing
//! about the configured toolchain.
//!
//! [`resolve_session_claim`] replaces the string coincidence with the actual signal: every
//! session's own [`ValidationSession::language`]. Exactly one configured session for a language
//! resolves unambiguously regardless of what it happens to be named (fixing the "no session at
//! all" half of #127). Two or more sessions for the same language with no explicit `target:` to
//! break the tie is a real ambiguity alef must not silently guess at -- `resolve_session_claim`
//! reports it as [`SessionClaim::Ambiguous`] so `session_preparation_error` can turn it into a
//! `SnippetStatus::Error` that fails every run, not just `--strict`, instead of it turning into a
//! `Pass`/`Fail` computed against whichever session happened to prepare first. ~keep

use crate::snippets::types::{Language, Snippet};
use std::collections::HashMap;

/// How a snippet's configured validation session resolves.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SessionClaim<'a> {
    /// No configured session shares this snippet's language; it validates outside any session.
    Unclaimed,
    /// Exactly one configured session claims this snippet -- its key into the sessions map.
    Claimed(&'a str),
    /// Two or more configured sessions share this snippet's language and no explicit `target`
    /// names one of them. Candidates are sorted for a stable diagnostic. ~keep
    Ambiguous(Vec<&'a str>),
}

/// Resolves the claim for `snippet` against `entries` -- every configured session keyed by its
/// target name, alongside a way to read that session's language back out. Generic over the
/// session value type so both `session_for`/`session_key` (routing, which must only ever consider
/// *successfully prepared* [`ValidationSession`]s) and `session_preparation_error` (ambiguity and
/// failed-session detection, which must consider every *configured* `SessionSpec` regardless of
/// whether it prepared) share this exact matching rule instead of two hand-copied ones drifting
/// apart. ~keep
///
/// An explicit `target:` (front matter or ledger-derived) is authoritative: it either names a
/// configured session (`Claimed`) or it does not (`Unclaimed`) -- it never falls through to a
/// language-matched candidate, because a snippet that named a target asked for that session
/// specifically, not "guess something in the same language". Only a snippet with no explicit
/// target considers every session whose own `language` matches; resolution only succeeds there
/// when exactly one candidate exists.
pub(super) fn resolve_session_claim<'a, T>(
    snippet: &Snippet,
    entries: &'a HashMap<String, T>,
    language_of: impl Fn(&T) -> Language,
) -> SessionClaim<'a> {
    if let Some(target) = snippet.metadata.target.as_deref() {
        let normalized = Language::normalize_session_target(target);
        return match entries.get_key_value(normalized.as_str()) {
            Some((key, _)) => SessionClaim::Claimed(key.as_str()),
            None => SessionClaim::Unclaimed,
        };
    }

    let mut candidates: Vec<&str> = entries
        .iter()
        .filter(|(_, value)| language_of(value) == snippet.language)
        .map(|(key, _)| key.as_str())
        .collect();
    candidates.sort_unstable();
    match candidates.as_slice() {
        [] => SessionClaim::Unclaimed,
        [only] => SessionClaim::Claimed(only),
        _ => SessionClaim::Ambiguous(candidates),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::session::ValidationSession;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::path::PathBuf;

    fn snippet(language: Language, target: Option<&str>) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("example.md"),
            language,
            title: None,
            code: String::new(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata {
                target: target.map(str::to_string),
                ..SnippetMetadata::default()
            },
            source_origin: SourceOrigin {
                path: PathBuf::from("example.md"),
                line: 1,
                block_index: 0,
            },
        }
    }

    fn session(language: Language) -> ValidationSession {
        ValidationSession {
            language,
            working_directory: PathBuf::new(),
            manifest: None,
            fingerprint: "fingerprint".into(),
            env: Default::default(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: Default::default(),
        }
    }

    /// The fix's headline case: a hand-written snippet (no `target:`) whose language has exactly
    /// one configured session, spelled nothing like the bare language name -- `node` for
    /// TypeScript, mirroring three of the four real consumer configs surveyed for #127. Before
    /// this module existed, `sessions.get("typescript")` missed entirely and the snippet
    /// validated with no session at all. ~keep
    #[test]
    fn a_single_same_language_session_claims_a_target_less_snippet_regardless_of_its_name() {
        let sessions = HashMap::from([("node".to_string(), session(Language::TypeScript))]);
        let snippet = snippet(Language::TypeScript, None);

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Claimed("node")
        );
    }

    /// The other headline case: two sessions share a language (a real consumer's
    /// `[sessions.typescript]` + `[sessions.wasm]`, both targeting TypeScript) and the snippet
    /// gives no explicit target to break the tie. The old fallback picked whichever session
    /// happened to be spelled like the bare language -- an accident of naming that silently
    /// starved the other session of any hand-written coverage. This must resolve as ambiguous,
    /// not as a silent pick. ~keep
    #[test]
    fn two_same_language_sessions_with_no_explicit_target_are_ambiguous() {
        let sessions = HashMap::from([
            ("typescript".to_string(), session(Language::TypeScript)),
            ("wasm".to_string(), session(Language::TypeScript)),
        ]);
        let snippet = snippet(Language::TypeScript, None);

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Ambiguous(vec!["typescript", "wasm"])
        );
    }

    /// An explicit `target:` still breaks the tie deterministically even when it would otherwise
    /// be ambiguous.
    #[test]
    fn an_explicit_target_resolves_an_otherwise_ambiguous_language() {
        let sessions = HashMap::from([
            ("typescript".to_string(), session(Language::TypeScript)),
            ("wasm".to_string(), session(Language::TypeScript)),
        ]);
        let snippet = snippet(Language::TypeScript, Some("wasm"));

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Claimed("wasm")
        );
    }

    /// An explicit target that names no configured session must not fall through to a
    /// same-language candidate -- that silent reroute is exactly the misreport #127 describes.
    /// It resolves as unclaimed, the same as a genuinely session-less language, rather than
    /// guessing at an unrelated session the snippet never asked for.
    #[test]
    fn an_explicit_target_naming_no_session_does_not_fall_back_to_a_language_match() {
        let sessions = HashMap::from([("node".to_string(), session(Language::TypeScript))]);
        let snippet = snippet(Language::TypeScript, Some("wasm"));

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Unclaimed
        );
    }

    #[test]
    fn no_configured_session_for_the_language_is_unclaimed() {
        let sessions = HashMap::from([("python".to_string(), session(Language::Python))]);
        let snippet = snippet(Language::TypeScript, None);

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Unclaimed
        );
    }
}
