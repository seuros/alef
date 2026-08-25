//! `missing_build_dependency` is the "ordering" half of alef #256: a consumer saw large
//! `Unavailable` counts, `Failed: 0`, for packages (`kotlin_android`, `wasm`) that were never
//! built. This function names that gap *before* a single toolchain runs, from configuration
//! alone -- so these tests pin exactly what counts as "no guaranteed build" and what does not.

use super::*;
use crate::snippets::types::{SnippetAnnotation, SnippetMetadata, SourceOrigin};
use std::path::PathBuf;

fn snippet(language: crate::snippets::types::Language) -> Snippet {
    Snippet {
        id: None,
        path: PathBuf::from("example.md"),
        language,
        title: None,
        code: String::new(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: PathBuf::from("example.md"),
            line: 1,
            block_index: 0,
        },
    }
}

fn session_with_before(language: crate::snippets::types::Language, before: Vec<&str>) -> SessionSpec {
    session_with_before_in(language, before, "/crate")
}

/// Same, but placing the session in a named package directory.
///
/// ~keep Ambiguity is decided by the WORKING DIRECTORY, not the language: two same-language
/// sessions over one directory describe one package and resolve deterministically (see
/// `session_resolution::resolve_session_claim`). A fixture that wants a genuinely ambiguous
/// claim must therefore give its sessions distinct directories — reusing the shared `/crate`
/// helper produces a *resolvable* claim and silently tests the opposite of what it says.
fn session_with_before_in(
    language: crate::snippets::types::Language,
    before: Vec<&str>,
    working_directory: &str,
) -> SessionSpec {
    SessionSpec {
        language,
        working_directory: PathBuf::from(working_directory),
        manifest: None,
        before: before.into_iter().map(str::to_string).collect(),
        env: Default::default(),
        include_paths: Vec::new(),
        rust_features: Vec::new(),
        rust_dependencies: Default::default(),
    }
}

/// The headline gap: a language with no configured session at all, validating at a level that
/// needs a compiled artifact, must be reported.
#[test]
fn a_language_with_no_configured_session_is_missing_a_build_dependency() {
    use crate::snippets::types::Language;

    let sessions = HashMap::new();
    let snippets = vec![snippet(Language::Kotlin)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::Compile);

    assert_eq!(missing.get(&Language::Kotlin), Some(&1));
}

/// A configured session with an empty `before` list has no mechanism to have produced the
/// artifact either -- same gap, different shape.
#[test]
fn a_session_with_an_empty_before_list_is_missing_a_build_dependency() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([("wasm".to_string(), session_with_before(Language::TypeScript, vec![]))]);
    let snippets = vec![snippet(Language::TypeScript)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::TypeCheck);

    assert_eq!(missing.get(&Language::TypeScript), Some(&1));
}

/// The negative control this whole check exists to not break: a session with a real `before`
/// build step must never be reported, however large the run.
#[test]
fn a_session_with_a_before_build_step_is_not_reported() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([(
        "go".to_string(),
        session_with_before(Language::Go, vec!["go build ./..."]),
    )]);
    let snippets = vec![snippet(Language::Go), snippet(Language::Go)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::Compile);

    assert!(missing.is_empty(), "a real before hook must clear the gap: {missing:?}");
}

/// The headline regression: `before = ["true"]` is non-empty, so the naive "is the list empty"
/// check this function used to run treated it exactly like a real build step. `true` is a shell
/// idiom that always exits 0 and produces nothing -- a session configured with only that command
/// has done nothing whatsoever toward building this language's artifact, and must be reported
/// exactly like an empty `before` list is on the line above.
#[test]
fn a_session_whose_only_before_command_is_a_no_op_is_missing_a_build_dependency() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([(
        "wasm".to_string(),
        session_with_before(Language::TypeScript, vec!["true"]),
    )]);
    let snippets = vec![snippet(Language::TypeScript)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::TypeCheck);

    assert_eq!(
        missing.get(&Language::TypeScript),
        Some(&1),
        "before = [\"true\"] must not satisfy the build guarantee: {missing:?}"
    );
}

/// `:` is the same no-op idiom as `true` under a different spelling, and must be caught the same
/// way.
#[test]
fn a_session_whose_only_before_command_is_a_colon_no_op_is_missing_a_build_dependency() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([("wasm".to_string(), session_with_before(Language::TypeScript, vec![":"]))]);
    let snippets = vec![snippet(Language::TypeScript)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::TypeCheck);

    assert_eq!(missing.get(&Language::TypeScript), Some(&1));
}

/// A `before` list can mix a no-op with a real build step (e.g. an author temporarily stubbing
/// one command while debugging another) -- as long as at least one configured command is not a
/// no-op, the session still has a real mechanism to have produced the artifact.
#[test]
fn a_session_with_a_real_command_alongside_a_no_op_is_not_reported() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([(
        "go".to_string(),
        session_with_before(Language::Go, vec!["true", "go build ./..."]),
    )]);
    let snippets = vec![snippet(Language::Go)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::Compile);

    assert!(
        missing.is_empty(),
        "a real command in the list must still count: {missing:?}"
    );
}

/// A real-world staging script is not a no-op: it is a single non-trivial shell invocation
/// (`bash scripts/stage_wasm_types.sh`), not one of the literal no-op idioms this function
/// recognizes syntactically. This function cannot verify the script actually builds anything --
/// only that the configured command is not the specific degenerate bypass `before = ["true"]`
/// demonstrated -- so a staging script must still satisfy the gate.
#[test]
fn a_staging_script_command_is_not_reported() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([(
        "wasm".to_string(),
        session_with_before(Language::TypeScript, vec!["bash ../../scripts/stage_wasm_types.sh"]),
    )]);
    let snippets = vec![snippet(Language::TypeScript)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::TypeCheck);

    assert!(
        missing.is_empty(),
        "a real script invocation must satisfy the gate: {missing:?}"
    );
}

/// `Syntax` never needs a compiled artifact -- a language with no session at all must not be
/// reported at that level, matching `snippet_validation_needs_build_artifacts`'s own
/// compile/typecheck/run boundary.
#[test]
fn syntax_level_never_counts_as_a_missing_build_dependency() {
    use crate::snippets::types::Language;

    let sessions = HashMap::new();
    let snippets = vec![snippet(Language::Kotlin)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::Syntax);

    assert!(
        missing.is_empty(),
        "syntax-level validation needs no artifact: {missing:?}"
    );
}

/// A snippet's own front-matter `level:` can clamp it back down to `Syntax` even when the run
/// requested `Run` -- `effective_validation_level` already accounts for that, and this function
/// must not re-derive its own, looser notion of "needs a build".
#[test]
fn a_snippet_declared_at_syntax_level_is_not_reported_even_under_a_stronger_request() {
    use crate::snippets::types::Language;

    let sessions = HashMap::new();
    let mut declared = snippet(Language::Kotlin);
    declared.metadata.level = Some(ValidationLevel::Syntax);

    let missing = missing_build_dependency(&[declared], &sessions, ValidationLevel::Run);

    assert!(
        missing.is_empty(),
        "a declared syntax-only snippet needs no artifact: {missing:?}"
    );
}

/// A `skip`-annotated snippet never reaches a toolchain at all, so it must not inflate a count
/// that is supposed to describe snippets a build gap would actually affect.
#[test]
fn a_skip_annotated_snippet_is_excluded() {
    use crate::snippets::types::{Language, SnippetAnnotationKind};

    let sessions = HashMap::new();
    let mut skipped = snippet(Language::Kotlin);
    skipped.annotation = Some(SnippetAnnotation {
        kind: SnippetAnnotationKind::Skip,
        reason: Some("documented elsewhere".to_string()),
    });

    let missing = missing_build_dependency(&[skipped], &sessions, ValidationLevel::Compile);

    assert!(
        missing.is_empty(),
        "a skipped snippet never runs, so it is not a build gap: {missing:?}"
    );
}

/// An ambiguous claim (two sessions, same language, no explicit target, and genuinely DIFFERENT
/// packages) is also unguaranteed: alef cannot say which of them -- if either -- produced the
/// artifact.
///
/// ~keep The two directories must differ. Same-language sessions sharing one working directory
/// describe one package and now resolve deterministically rather than reporting ambiguity, so a
/// fixture built on the shared `/crate` helper would assert the opposite of its own name.
#[test]
fn an_ambiguous_claim_is_reported_as_missing_a_build_dependency() {
    use crate::snippets::types::Language;

    let sessions = HashMap::from([
        (
            "typescript".to_string(),
            session_with_before_in(Language::TypeScript, vec!["pnpm build"], "/packages/typescript"),
        ),
        (
            "wasm".to_string(),
            session_with_before_in(Language::TypeScript, vec!["wasm-pack build"], "/packages/wasm"),
        ),
    ]);
    let snippets = vec![snippet(Language::TypeScript)];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::Compile);

    assert_eq!(missing.get(&Language::TypeScript), Some(&1));
}

/// The per-language breakdown must count every affected language independently, not collapse
/// them into a single total -- a consumer needs to know which packages to build.
#[test]
fn counts_are_kept_separate_per_language() {
    use crate::snippets::types::Language;

    let sessions = HashMap::new();
    let snippets = vec![
        snippet(Language::Kotlin),
        snippet(Language::Kotlin),
        snippet(Language::TypeScript),
    ];

    let missing = missing_build_dependency(&snippets, &sessions, ValidationLevel::Compile);

    assert_eq!(missing.get(&Language::Kotlin), Some(&2));
    assert_eq!(missing.get(&Language::TypeScript), Some(&1));
}
