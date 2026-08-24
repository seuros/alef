//! Resolves which configured validation session -- if any -- claims a snippet, as a single
//! function every dispatch path and `session_preparation_error` share.
//!
//! ## Alef defect #127
//!
//! A hand-written snippet carries no `metadata.target` (that field is only ever populated from
//! front matter or from a generated snippet's coverage ledger), so before this module existed the
//! fallback was a literal string lookup: `sessions.get(&snippet.language.to_string())`, i.e. "is
//! there a configured session target spelled exactly like the bare language name". That only ever
//! matched by naming coincidence. One consumer's `alef.toml` configures both
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
//! `Pass`/`Fail` computed against whichever session happened to prepare first.
//!
//! ## Alef issue #255
//!
//! `Language` is a many-to-one projection of a configured target:
//! `Language::from_session_target` maps both `kotlin` and `kotlin_android` to `Language::Kotlin`,
//! and `typescript`/`node`/`wasm` all map to `Language::TypeScript`. A consumer whose Kotlin and
//! Kotlin Android bindings share one Gradle directory (the ordinary case -- Android reuses the JVM
//! package) configures both targets over that one directory, and a consumer targeting Node and
//! WASM from one TypeScript package configures all three. Candidate matching that keyed purely on
//! `language_of` treated these as competing sessions and reported every target-less snippet in
//! that language as [`SessionClaim::Ambiguous`], even though every candidate validates the exact
//! same physical package and any one of them would produce an identical result.
//!
//! [`SessionIdentity::working_directory`] is the fix: candidates that share one language now only
//! count as genuinely ambiguous when they also validate different working directories. Same
//! directory collapses to a single, deterministic [`SessionClaim::Claimed`] (the alphabetically
//! first target name) instead of an ambiguity report -- session identity is the physical package a
//! session validates, not the `Language` its target string happens to resolve to. Two candidates
//! that really do point at different directories are still reported `Ambiguous`, unchanged. ~keep

use crate::snippets::types::{Language, Snippet};
use std::collections::HashMap;
use std::path::Path;

/// A configured or prepared session's physical identity: the working directory it actually
/// validates. `ValidationSession` and `SessionSpec` both carry this field directly; this trait
/// lets [`resolve_session_claim`] read it generically from whichever the caller passes, the same
/// way `language_of` already does for [`Language`]. ~keep
pub(super) trait SessionIdentity {
    fn working_directory(&self) -> &Path;
}

impl SessionIdentity for crate::snippets::session::ValidationSession {
    fn working_directory(&self) -> &Path {
        &self.working_directory
    }
}

impl SessionIdentity for crate::snippets::session::SessionSpec {
    fn working_directory(&self) -> &Path {
        &self.working_directory
    }
}

/// How a snippet's configured validation session resolves.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SessionClaim<'a> {
    /// No configured session shares this snippet's language; it validates outside any session.
    Unclaimed,
    /// Exactly one configured session claims this snippet -- its key into the sessions map. Also
    /// returned when multiple same-language candidates all share one working directory (alef
    /// issue #255): they are one physical session under different config aliases, so the
    /// alphabetically first key is picked deterministically rather than reported as ambiguous.
    Claimed(&'a str),
    /// Two or more configured sessions share this snippet's language, validate different working
    /// directories, and no explicit `target` names one of them. Candidates are sorted for a
    /// stable diagnostic. ~keep
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
/// when exactly one candidate exists, or when every candidate shares one working directory (see
/// [`SessionIdentity`] and the `#255` module doc). ~keep
pub(super) fn resolve_session_claim<'a, T: SessionIdentity>(
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
        _ if distinct_working_directory_count(entries, &candidates) <= 1 => SessionClaim::Claimed(candidates[0]),
        _ => SessionClaim::Ambiguous(candidates),
    }
}

/// How many distinct working directories `candidates` actually validate. A count of `1` (or `0`,
/// unreachable here since `candidates` is never empty in the caller) means every same-language
/// candidate is the same physical session under a different config alias -- not a real ambiguity.
/// A count above `1` means the candidates are genuinely different packages and a `target:` is
/// needed to break the tie. ~keep
fn distinct_working_directory_count<'a, T: SessionIdentity>(
    entries: &'a HashMap<String, T>,
    candidates: &[&str],
) -> usize {
    let mut directories: Vec<&'a Path> = candidates
        .iter()
        .filter_map(|key| entries.get(*key))
        .map(SessionIdentity::working_directory)
        .collect();
    directories.sort_unstable();
    directories.dedup();
    directories.len()
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

    fn session(language: Language, working_directory: &str) -> ValidationSession {
        ValidationSession {
            language,
            working_directory: PathBuf::from(working_directory),
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
        let sessions = HashMap::from([("node".to_string(), session(Language::TypeScript, "packages/node"))]);
        let snippet = snippet(Language::TypeScript, None);

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Claimed("node")
        );
    }

    /// The other headline case: two sessions share a language (a consumer's real
    /// `[sessions.typescript]` + `[sessions.wasm]`, both targeting TypeScript) but validate
    /// *different* packages, and the snippet gives no explicit target to break the tie. The old
    /// fallback picked whichever session happened to be spelled like the bare language -- an
    /// accident of naming that silently starved the other session of any hand-written coverage.
    /// This must resolve as ambiguous, not as a silent pick. Distinct from alef issue #255's
    /// same-directory collapse below: these two candidates genuinely disagree about which package
    /// they validate. ~keep
    #[test]
    fn two_same_language_sessions_over_different_directories_are_ambiguous() {
        let sessions = HashMap::from([
            (
                "typescript".to_string(),
                session(Language::TypeScript, "packages/typescript"),
            ),
            ("wasm".to_string(), session(Language::TypeScript, "packages/wasm")),
        ]);
        let snippet = snippet(Language::TypeScript, None);

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Ambiguous(vec!["typescript", "wasm"])
        );
    }

    /// Alef issue #255: `kotlin` and `kotlin_android` both resolve to `Language::Kotlin` via
    /// `Language::from_session_target`, and a consumer whose Kotlin Android bindings share the
    /// same Gradle directory as its Kotlin bindings configures both targets over that one
    /// directory. Session identity is the working directory, not the resolved `Language` -- this
    /// must collapse to one claimed session, not an ambiguity report. ~keep
    #[test]
    fn kotlin_and_kotlin_android_over_one_directory_collapse_to_one_session() {
        let sessions = HashMap::from([
            ("kotlin".to_string(), session(Language::Kotlin, "packages/kotlin")),
            (
                "kotlin_android".to_string(),
                session(Language::Kotlin, "packages/kotlin"),
            ),
        ]);
        let mut candidates: Vec<&str> = sessions.keys().map(String::as_str).collect();
        candidates.sort_unstable();
        assert_eq!(
            distinct_working_directory_count(&sessions, &candidates),
            1,
            "kotlin and kotlin_android must resolve to a single physical session, not two"
        );

        let snippet = snippet(Language::Kotlin, None);
        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Claimed("kotlin")
        );
    }

    /// The other #255 collapse: `typescript`, `node`, and `wasm` all resolve to
    /// `Language::TypeScript` via `Language::from_session_target`, and a consumer targeting Node
    /// and WASM from one TypeScript package configures all three over the same directory. This
    /// must collapse to one claimed session, not three competing candidates. ~keep
    #[test]
    fn typescript_node_and_wasm_over_one_package_collapse_to_one_session() {
        let sessions = HashMap::from([
            (
                "typescript".to_string(),
                session(Language::TypeScript, "packages/typescript"),
            ),
            ("node".to_string(), session(Language::TypeScript, "packages/typescript")),
            ("wasm".to_string(), session(Language::TypeScript, "packages/typescript")),
        ]);
        let mut candidates: Vec<&str> = sessions.keys().map(String::as_str).collect();
        candidates.sort_unstable();
        assert_eq!(
            distinct_working_directory_count(&sessions, &candidates),
            1,
            "typescript, node, and wasm must resolve to a single physical session, not three"
        );

        let snippet = snippet(Language::TypeScript, None);
        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Claimed("node")
        );
    }

    /// An explicit `target:` still breaks the tie deterministically even when it would otherwise
    /// be ambiguous.
    #[test]
    fn an_explicit_target_resolves_an_otherwise_ambiguous_language() {
        let sessions = HashMap::from([
            (
                "typescript".to_string(),
                session(Language::TypeScript, "packages/typescript"),
            ),
            ("wasm".to_string(), session(Language::TypeScript, "packages/wasm")),
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
        let sessions = HashMap::from([("node".to_string(), session(Language::TypeScript, "packages/node"))]);
        let snippet = snippet(Language::TypeScript, Some("wasm"));

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Unclaimed
        );
    }

    #[test]
    fn no_configured_session_for_the_language_is_unclaimed() {
        let sessions = HashMap::from([("python".to_string(), session(Language::Python, "packages/python"))]);
        let snippet = snippet(Language::TypeScript, None);

        assert_eq!(
            resolve_session_claim(&snippet, &sessions, |session| session.language),
            SessionClaim::Unclaimed
        );
    }
}
