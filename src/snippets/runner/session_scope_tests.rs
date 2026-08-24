//! `sessions_needed_for_preparation` is the fix for the other half of the `--lang` complaint
//! routed alongside GH #256: `alef snippets check --lang go` filtered *discovery* to Go snippets,
//! but `run_validation` still handed `prepare_sessions_isolated` every configured session
//! regardless, running every `before` hook in the crate's config -- Kotlin, Swift, and every other
//! language -- on a single-language diagnostic.
//!
//! These tests pin the fix at the one place it is safe to observe without spawning a real
//! toolchain: what `sessions_needed_for_preparation` decides to prepare, given a snippet slice and
//! the full configured session map. The negative control (`unfiltered run still needs every
//! session`) matters as much as the positive one -- a fix that stops preparing sessions
//! unconditionally would pass a naive "narrowed run only touches one session" test while breaking
//! every full run.

use super::*;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use std::path::PathBuf;

fn snippet(language: crate::snippets::types::Language, target: Option<&str>) -> Snippet {
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

fn session(language: crate::snippets::types::Language, working_directory: &str) -> SessionSpec {
    SessionSpec {
        language,
        working_directory: PathBuf::from(working_directory),
        manifest: None,
        before: vec!["build".to_string()],
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    }
}

/// The headline fix: a run whose snippets are all one language must not prepare a session for a
/// language none of them use.
#[test]
fn a_lang_filtered_run_prepares_only_the_matching_session() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        ("go".to_string(), session(Language::Go, "/crate/go")),
        ("kotlin".to_string(), session(Language::Kotlin, "/crate/kotlin")),
        ("swift".to_string(), session(Language::Swift, "/crate/swift")),
    ]);
    let snippets = vec![snippet(Language::Go, None)];

    let needed = sessions_needed_for_preparation(&snippets, &sessions);

    assert_eq!(needed.len(), 1, "expected only the go session, got {needed:?}");
    assert!(needed.contains_key("go"));
}

/// Negative control: an unfiltered run (every configured language represented among the
/// snippets) must still prepare every session. A fix that just stopped preparing sessions
/// wholesale would pass the positive test above and silently break this one.
#[test]
fn an_unfiltered_run_still_needs_every_configured_session() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        ("go".to_string(), session(Language::Go, "/crate/go")),
        ("kotlin".to_string(), session(Language::Kotlin, "/crate/kotlin")),
        ("swift".to_string(), session(Language::Swift, "/crate/swift")),
    ]);
    let snippets = vec![
        snippet(Language::Go, None),
        snippet(Language::Kotlin, None),
        snippet(Language::Swift, None),
    ];

    let needed = sessions_needed_for_preparation(&snippets, &sessions);

    assert_eq!(needed.len(), 3, "expected every session, got {needed:?}");
}

/// A language with no snippet in this run and no shared working directory with a needed session
/// must be dropped entirely -- the actual perf fix.
#[test]
fn an_unclaimed_language_with_its_own_working_directory_is_dropped() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        ("python".to_string(), session(Language::Python, "/crate/python")),
        ("zig".to_string(), session(Language::Zig, "/crate/zig")),
    ]);
    let snippets = vec![snippet(Language::Python, None)];

    let needed = sessions_needed_for_preparation(&snippets, &sessions);

    assert!(
        !needed.contains_key("zig"),
        "zig has no claim and no shared directory: {needed:?}"
    );
}

/// The purge-safety guard: two sessions sharing one working directory must both be prepared even
/// when only one of them is actually claimed by this run's snippets. Preparing only the claimed
/// one would leave the other's live fingerprint directory looking abandoned to
/// `purge_stale_session_scratch`, which deletes whatever isn't live -- destroying a sibling
/// session's build cache to speed up an unrelated run.
#[test]
fn a_session_sharing_a_working_directory_with_a_needed_one_is_kept_even_when_unclaimed() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        (
            "typescript".to_string(),
            session(Language::TypeScript, "/crate/bindings"),
        ),
        ("wasm".to_string(), session(Language::Rust, "/crate/bindings")),
    ]);
    let snippets = vec![snippet(Language::TypeScript, None)];

    let needed = sessions_needed_for_preparation(&snippets, &sessions);

    assert_eq!(
        needed.len(),
        2,
        "the cohabiting wasm session must be kept to protect its scratch: {needed:?}"
    );
}

/// An explicit `target:` still resolves correctly through the filter -- the claim comes from
/// `session_resolution::resolve_session_claim`, not a bare language match.
#[test]
fn an_explicit_target_claims_its_named_session_even_when_the_language_matches_another_too() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        ("typescript".to_string(), session(Language::TypeScript, "/crate/node")),
        ("wasm".to_string(), session(Language::TypeScript, "/crate/wasm")),
    ]);
    let snippets = vec![snippet(Language::TypeScript, Some("wasm"))];

    let needed = sessions_needed_for_preparation(&snippets, &sessions);

    assert_eq!(
        needed.len(),
        1,
        "expected only the explicitly targeted session: {needed:?}"
    );
    assert!(needed.contains_key("wasm"));
}

/// An ambiguous claim (two same-language sessions, no explicit target) keeps every candidate --
/// conservative, since the claim itself is unresolved and dropping one candidate is not safe.
#[test]
fn an_ambiguous_claim_keeps_every_candidate_session() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        ("typescript".to_string(), session(Language::TypeScript, "/crate/node")),
        ("wasm".to_string(), session(Language::TypeScript, "/crate/wasm")),
    ]);
    let snippets = vec![snippet(Language::TypeScript, None)];

    let needed = sessions_needed_for_preparation(&snippets, &sessions);

    assert_eq!(
        needed.len(),
        2,
        "an ambiguous claim must not silently drop a candidate: {needed:?}"
    );
}
