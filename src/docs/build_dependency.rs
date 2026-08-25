//! Pre-flight gate for the "ordering" half of alef #256: a consumer's clean validation run
//! reported large `Unavailable` counts, `Failed: 0`, for packages (`kotlin_android`, `wasm`) that
//! were never built. Their snippets validated against artifacts no stage in the invoked pipeline
//! had produced -- discoverable from configuration alone, before a single toolchain runs, via
//! `crate::snippets::runner::missing_build_dependency`.
//!
//! `enforce_snippet_summary` (this module's parent) already attributes and reports the
//! after-the-fact shape of this gap (`unresolved_dependency`, a validator that ran, failed on a
//! missing import, and got reclassified) -- and deliberately does not fail non-strict runs on it,
//! because `alef docs`/`alef all` structurally cannot have run the build themselves (see
//! `bin_cli::all_commands::warn_if_snippet_validation_needs_build`'s doc). This gate is narrower
//! and earlier: it only fires when a language has *no configured mechanism at all* to have
//! produced its artifact (no session, an ambiguous one, or one with an empty `before` list), and
//! it runs before `run_validation` spends any toolchain time on snippets already known to be
//! doomed. Under `--strict` that absence of any build mechanism is exactly the "pipeline
//! guarantee" gap alef #256 asks for: a run must not be able to validate zero real artifacts for
//! a language and still read as clean.

use crate::snippets::runner::missing_build_dependency;
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{Language, Snippet, ValidationLevel};
use std::collections::{BTreeMap, HashMap};

/// Warns (always) and bails (under `strict`) when `snippets` contains results that need a
/// compiled artifact but have no configured session that could plausibly have produced one in
/// this invocation. See the module doc for why this is narrower than, and does not replace,
/// `enforce_snippet_summary`'s `unresolved_dependency` handling.
///
/// # Errors
///
/// Returns an error when `strict` is set and at least one language has no build guarantee.
pub(super) fn enforce_build_dependency(
    crate_name: &str,
    strict: bool,
    snippets: &[Snippet],
    sessions: &HashMap<String, SessionSpec>,
    requested_level: ValidationLevel,
) -> anyhow::Result<()> {
    let missing = missing_build_dependency(snippets, sessions, requested_level);
    if missing.is_empty() {
        return Ok(());
    }
    let total: usize = missing.values().sum();
    let breakdown = format_breakdown(&missing);
    tracing::warn!(
        missing_build_dependency = total,
        requested_level = %requested_level,
        "[{crate_name}] docs.snippets.validation_level = \"{requested_level}\" requires a compiled artifact, but \
         {total} snippet(s) have no configured session guaranteed to have produced one before this run reads it \
         ({breakdown}) -- add a `before` build step under `docs.snippets.sessions.<target>` for the affected \
         language(s), or run `alef build` first. This is an ordering gap, not a snippet defect."
    );
    if strict {
        anyhow::bail!(
            "strict snippet validation failed for crate `{crate_name}`: {total} snippet(s) validate at \
             \"{requested_level}\" with no pipeline-guaranteed build for their language ({breakdown}) -- this is \
             an ordering gap: configure `docs.snippets.sessions.<target>.before` to build the artifact before \
             validation, or run `alef build` first"
        );
    }
    Ok(())
}

fn format_breakdown(missing: &BTreeMap<Language, usize>) -> String {
    missing
        .iter()
        .map(|(language, count)| format!("{language} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::path::PathBuf;

    fn snippet(language: Language) -> Snippet {
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

    /// The headline regression: a crate with a build-requiring level and a language with no
    /// configured session must fail under `--strict`, naming the language and the count.
    #[test]
    fn strict_bails_when_a_language_has_no_build_guarantee() {
        let sessions = HashMap::new();
        let snippets = vec![snippet(Language::Kotlin)];

        let error = enforce_build_dependency("fixture", true, &snippets, &sessions, ValidationLevel::Compile)
            .expect_err("no session for kotlin must fail strict");

        let message = error.to_string();
        assert!(message.contains("ordering gap"), "{message}");
        assert!(message.contains("kotlin 1"), "{message}");
    }

    /// Non-strict must not fail on this gap -- `alef docs`/`alef all` cannot have built the
    /// artifact themselves, so this stays a loud warning, matching `enforce_snippet_summary`'s
    /// existing non-strict treatment of `unresolved_dependency`.
    #[test]
    fn non_strict_does_not_bail_on_the_same_gap() {
        let sessions = HashMap::new();
        let snippets = vec![snippet(Language::Kotlin)];

        let result = enforce_build_dependency("fixture", false, &snippets, &sessions, ValidationLevel::Compile);

        assert!(result.is_ok(), "non-strict must warn, not bail: {result:?}");
    }

    /// Negative control: a language with a real `before` build step must never bail, however
    /// strict the run.
    #[test]
    fn strict_does_not_bail_when_every_language_has_a_build_guarantee() {
        let sessions = HashMap::from([(
            "kotlin".to_string(),
            SessionSpec {
                language: Language::Kotlin,
                working_directory: PathBuf::from("/crate"),
                manifest: None,
                before: vec!["./gradlew build".to_string()],
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        )]);
        let snippets = vec![snippet(Language::Kotlin)];

        let result = enforce_build_dependency("fixture", true, &snippets, &sessions, ValidationLevel::Compile);

        assert!(
            result.is_ok(),
            "a configured before hook must satisfy the gate: {result:?}"
        );
    }

    /// The headline regression at the `--strict` boundary: `before = ["true"]` is a session that
    /// exists and has a non-empty `before` list, so it must not be able to buy its way past this
    /// gate. `true` always exits 0 and builds nothing -- a run that lets this pass under
    /// `--strict` reads as clean while having validated zero real artifacts for the language,
    /// exactly the alef #256 shape this gate exists to catch.
    #[test]
    fn strict_bails_when_the_only_before_command_is_a_no_op() {
        let sessions = HashMap::from([(
            "kotlin".to_string(),
            SessionSpec {
                language: Language::Kotlin,
                working_directory: PathBuf::from("/crate"),
                manifest: None,
                before: vec!["true".to_string()],
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        )]);
        let snippets = vec![snippet(Language::Kotlin)];

        let error = enforce_build_dependency("fixture", true, &snippets, &sessions, ValidationLevel::Compile)
            .expect_err("before = [\"true\"] must not satisfy strict mode");

        let message = error.to_string();
        assert!(message.contains("ordering gap"), "{message}");
        assert!(message.contains("kotlin 1"), "{message}");
    }

    /// Negative control: `Syntax`-level validation needs no artifact at all, so a crate with no
    /// sessions configured at all must never bail on this gate.
    #[test]
    fn strict_does_not_bail_at_syntax_level() {
        let sessions = HashMap::new();
        let snippets = vec![snippet(Language::Kotlin)];

        let result = enforce_build_dependency("fixture", true, &snippets, &sessions, ValidationLevel::Syntax);

        assert!(result.is_ok(), "syntax-level validation needs no artifact: {result:?}");
    }
}
