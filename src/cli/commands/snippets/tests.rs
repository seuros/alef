//! Unit tests for the `alef snippets` subcommand.
//!
//! Split out of `snippets.rs` so that file stays under the 1,000-line cap this repository sets
//! for CLI sources; the module keeps `use super::*` and is otherwise unchanged. ~keep

use super::*;

/// `--lang` narrows a run to one backend's snippets, so an unrecognised value has to be
/// reported rather than dropped: dropping it leaves an empty-but-`Some` filter, which reads to
/// discovery as "match nothing" and fails the run naming the directories, not the typo. ~keep
#[test]
fn a_language_filter_keeps_every_recognised_fence_tag() {
    let requested = ["go".to_string(), "typescript".to_string(), "zig".to_string()];
    let parsed = parse_language_filter(Some(&requested)).expect("a filter was requested");

    assert_eq!(
        parsed.recognised,
        vec![Language::Go, Language::TypeScript, Language::Zig]
    );
    assert!(parsed.unrecognised.is_empty(), "no recognised tag may be rejected");
}

/// The names in an `alef.toml` session table are session targets, not fence tags, and the two
/// vocabularies differ. `--lang kotlin_android` has to reach the Kotlin snippets, or the only
/// name a user has for that session selects nothing. ~keep
#[test]
fn a_language_filter_accepts_session_target_names_as_well_as_fence_tags() {
    let requested = [
        "kotlin_android".to_string(),
        "kotlin-android".to_string(),
        "node".to_string(),
        "wasm".to_string(),
    ];
    let parsed = parse_language_filter(Some(&requested)).expect("a filter was requested");

    assert!(parsed.unrecognised.is_empty(), "session target names must resolve");
    assert_eq!(
        parsed.recognised,
        vec![Language::Kotlin, Language::TypeScript],
        "aliases collapse to one entry each rather than repeating a language"
    );
}

#[test]
fn a_language_filter_reports_an_unrecognised_tag_instead_of_dropping_it() {
    let requested = ["go".to_string(), "nosuchlang".to_string()];
    let parsed = parse_language_filter(Some(&requested)).expect("a filter was requested");

    assert_eq!(parsed.recognised, vec![Language::Go]);
    assert_eq!(parsed.unrecognised, vec!["nosuchlang".to_string()]);
    assert!(
        reject_unrecognised_languages(Some(&parsed)).is_err(),
        "an unrecognised tag must fail the run, not narrow it silently"
    );
}

#[test]
fn no_language_argument_means_no_filter_at_all() {
    assert!(parse_language_filter(None).is_none());
    assert!(reject_unrecognised_languages(None).is_ok());
}

/// `required_languages` takes a snippet fence tag, but the name a user reaches for first is the
/// directory/session target name they just wrote under `[crates.kotlin_android]` -- and that
/// name used to be rejected outright with no hint a different vocabulary was expected. ~keep
#[test]
fn required_language_accepts_a_session_target_name_as_well_as_a_fence_tag() {
    assert_eq!(resolve_required_language("kotlin"), Ok(Language::Kotlin));
    assert_eq!(resolve_required_language("kotlin_android"), Ok(Language::Kotlin));
    assert_eq!(resolve_required_language("kotlin-android"), Ok(Language::Kotlin));
    assert_eq!(resolve_required_language("node"), Ok(Language::TypeScript));
}

/// The other half: a value that resolves to neither vocabulary must still fail loudly, naming
/// both accepted forms, rather than resolving to `Unknown` and being silently dropped from the
/// comparison. ~keep
#[test]
fn required_language_names_both_accepted_vocabularies_when_it_rejects_a_value() {
    let error = resolve_required_language("nosuchlang").expect_err("nosuchlang is not a language");
    assert!(
        error.contains("nosuchlang"),
        "error should name the rejected value: {error}"
    );
    assert!(
        error.contains("fence tag") && error.contains("session target"),
        "error should name both accepted vocabularies: {error}"
    );
}

/// `alef snippets gaps --required-languages` used to resolve an unrecognised entry to
/// `Language::Unknown` and silently filter it out of the comparison instead of failing the
/// run -- exactly the same shape as the language-filter defect
/// `a_language_filter_reports_an_unrecognised_tag_instead_of_dropping_it` covers for `--lang`.
/// ~keep
#[test]
fn an_unrecognised_required_language_fails_the_gaps_run_instead_of_being_dropped() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = directory.path().join("snippets");
    std::fs::create_dir_all(&snippets).expect("snippet directory");

    let required = ["nosuchlang".to_string()];
    let code = run_gaps(&GapInvocation {
        snippet_dirs: std::slice::from_ref(&snippets),
        docs_dirs: &[],
        required_languages: Some(&required),
        include_base_paths: &[],
        strict: false,
    });

    assert!(
        !is_success(code),
        "an unrecognised --required-languages value must fail the run, not silently narrow it"
    );
}

/// A session target name in `--required-languages` (`kotlin_android`) must reach the same
/// language-parity comparison a fence tag (`kotlin`) would -- proving `resolve_required_language`
/// is actually wired into `run_gaps`, not just unit-tested in isolation.
#[test]
fn a_session_target_name_in_required_languages_drives_the_gaps_parity_check() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = ledger_backed_snippet_tree(directory.path(), &["python"]);

    let required = ["kotlin_android".to_string()];
    let code = run_gaps(&GapInvocation {
        snippet_dirs: std::slice::from_ref(&snippets),
        docs_dirs: &[],
        required_languages: Some(&required),
        include_base_paths: &[],
        strict: false,
    });

    assert!(
        !is_success(code),
        "kotlin_android must resolve to the Kotlin fence tag and be compared against the tree, \
         which has no kotlin variant at all -- that is a real gap"
    );
}

#[test]
fn strict_coverage_rejects_every_non_validation_status() {
    assert!(is_incomplete_status(SnippetStatus::Skip));
    assert!(is_incomplete_status(SnippetStatus::Unavailable));
    assert!(is_incomplete_status(SnippetStatus::Downgraded));
    assert!(!is_incomplete_status(SnippetStatus::Pass));
}

fn audit_and_gaps_without_a_docs_surface(directory: &Path, strict: bool) -> (bool, bool) {
    let snippets = directory.join("snippets");
    std::fs::create_dir_all(&snippets).expect("snippet directory");
    std::fs::write(snippets.join("weird.md"), "```gibberish\nvalue\n```\n").expect("write snippet");
    let snippet_directories = [snippets];

    run_configured_audit_and_gaps(&ConfiguredCheckInputs {
        snippet_directories: &snippet_directories,
        docs_directories: &[],
        include_base_paths: &[],
        configured_include_base_paths: &[],
        required_languages: &[],
        exclude: &[],
        readme: None,
        content_collections: &std::collections::BTreeMap::new(),
        workspace_root: directory,
        require_frontmatter: false,
        strict,
    })
    .expect("audit and gap pass")
}

#[test]
fn configured_audit_is_skipped_without_a_docs_surface() {
    let directory = tempfile::tempdir().expect("temp directory");
    let (audit_failure, gap_failure) = audit_and_gaps_without_a_docs_surface(directory.path(), false);

    assert!(
        !audit_failure,
        "a snippets-only config has no documentation surface to audit, so an unknown fence tag \
         must not fail the gate — `docs/mod.rs::validate_snippets` skips audit the same way"
    );
    assert!(
        !gap_failure,
        "gaps are meaningless without docs dirs or required languages, so a non-strict run stays green"
    );
}

/// The `snippets check` half of the same defect: with neither `docs_dirs` nor
/// `required_languages` configured the gap pass is skipped outright, and it used to report
/// "no failure" without a word about it — a strict CI gate passing on a check that never ran.
/// ~keep
#[test]
fn a_strict_check_fails_when_the_gap_pass_was_skipped_for_want_of_configuration() {
    let directory = tempfile::tempdir().expect("temp directory");
    let (_, gap_failure) = audit_and_gaps_without_a_docs_surface(directory.path(), true);

    assert!(
        gap_failure,
        "a strict run must not pass on a gap check that compared nothing"
    );
}

#[test]
fn readme_snippet_mappings_count_as_references_for_the_strict_gate() {
    let directory = tempfile::tempdir().expect("temp directory");
    let snippets = directory.path().join("snippets");
    let docs = directory.path().join("docs");
    std::fs::create_dir_all(snippets.join("python")).expect("snippet directory");
    std::fs::create_dir_all(&docs).expect("docs directory");
    std::fs::write(snippets.join("python/hello.md"), "```python\nvalue = 1\n```\n").expect("write snippet");
    let snippet_directories = [snippets];
    let docs_directories = [docs];
    let content_collections = std::collections::BTreeMap::new();
    let readme = crate::core::config::ReadmeConfig {
        template_dir: None,
        snippets_dir: Some(PathBuf::from("snippets")),
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::from([(
            "python".to_string(),
            serde_json::json!({ "snippets": ["hello.md"] }),
        )]),
        targets: std::collections::HashMap::new(),
    };

    let (audit_failure, gap_failure) = run_configured_audit_and_gaps(&ConfiguredCheckInputs {
        snippet_directories: &snippet_directories,
        docs_directories: &docs_directories,
        include_base_paths: &docs_directories,
        configured_include_base_paths: &docs_directories,
        required_languages: &[],
        exclude: &[],
        readme: Some(&readme),
        content_collections: &content_collections,
        workspace_root: directory.path(),
        require_frontmatter: false,
        strict: true,
    })
    .expect("audit and gap pass");

    assert!(!audit_failure);
    assert!(
        !gap_failure,
        "a snippet named by [crates.readme.languages.*].snippets is referenced even though no \
         documentation page `--8<--`-includes it"
    );

    let (_, gap_failure_without_readme) = run_configured_audit_and_gaps(&ConfiguredCheckInputs {
        snippet_directories: &snippet_directories,
        docs_directories: &docs_directories,
        include_base_paths: &docs_directories,
        configured_include_base_paths: &docs_directories,
        required_languages: &[],
        exclude: &[],
        readme: None,
        content_collections: &content_collections,
        workspace_root: directory.path(),
        require_frontmatter: false,
        strict: true,
    })
    .expect("audit and gap pass");

    assert!(
        gap_failure_without_readme,
        "without the README source the same snippet reads as unreferenced, so this test would \
         pass vacuously if the reference sources were dropped"
    );
}

#[test]
fn resolved_roots_drop_excluded_prefixes() {
    let root = Path::new("/workspace");
    let excluded = [root.join("snippets/vendored")];

    let resolved = resolved_roots(
        root,
        &[PathBuf::from("snippets"), PathBuf::from("snippets/vendored")],
        &excluded,
    );

    assert_eq!(resolved, vec![root.join("snippets")]);
}

#[test]
fn generated_coverage_manifest_exposes_missing_cells() {
    let directory = tempfile::tempdir().expect("temp directory");
    let ledger = crate::e2e::snippets::SnippetCoverageLedger {
        expected: vec![crate::e2e::snippets::SnippetCoverageKey {
            fixture_id: "extension_only".into(),
            language: "python".into(),
        }],
        missing: vec![crate::e2e::snippets::MissingSnippet {
            key: crate::e2e::snippets::SnippetCoverageKey {
                fixture_id: "extension_only".into(),
                language: "python".into(),
            },
            reason: "no compatible recipe".into(),
        }],
        ..Default::default()
    };
    std::fs::write(
        directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec(&ledger).expect("serialize ledger"),
    )
    .expect("write ledger");

    let missing = missing_generated_snippets(&[directory.path().to_path_buf()]).expect("read ledger");
    assert_eq!(missing, ledger.missing);
}

#[test]
fn configured_sessions_accept_binding_targets_and_reject_unknown_keys() {
    let root = std::env::current_dir()
        .expect("current directory")
        .join("neutral-workspace");
    let mut config = crate::core::config::DocsSnippetsConfig::default();
    config.sessions.insert(
        "wasm".into(),
        crate::core::config::output::DocsSnippetSessionConfig {
            cwd: "bindings/wasm".into(),
            ..Default::default()
        },
    );
    let sessions = configured_sessions(&config, &root, &[]).expect("known target");
    assert_eq!(sessions["wasm"].language, Language::TypeScript);
    assert_eq!(sessions["wasm"].working_directory, root.join("bindings/wasm"));

    config.sessions.insert(
        "unsupported-runtime".into(),
        crate::core::config::output::DocsSnippetSessionConfig::default(),
    );
    assert!(configured_sessions(&config, &root, &[]).is_err());
}

#[test]
fn configured_rust_session_enables_crate_features_so_gated_modules_resolve() {
    let root = std::env::current_dir()
        .expect("current directory")
        .join("neutral-workspace");
    let mut config = crate::core::config::DocsSnippetsConfig::default();
    config.sessions.insert(
        "rust".into(),
        crate::core::config::output::DocsSnippetSessionConfig {
            cwd: "crates/sample-core".into(),
            rust_features: vec!["telemetry".into()],
            ..Default::default()
        },
    );
    config.sessions.insert(
        "wasm".into(),
        crate::core::config::output::DocsSnippetSessionConfig {
            cwd: "bindings/wasm".into(),
            ..Default::default()
        },
    );
    let crate_features = vec!["plugins".to_string(), "telemetry".to_string()];

    let sessions = configured_sessions(&config, &root, &crate_features).expect("known targets");

    assert_eq!(sessions["rust"].language, Language::Rust);
    assert_eq!(
        sessions["rust"].rust_features,
        vec!["plugins".to_string(), "telemetry".to_string()],
        "a Rust snippet session must build the path dependency with the crate's declared features, \
         otherwise snippets importing a feature-gated module fail with `unresolved import`"
    );
    assert!(
        sessions["wasm"].rust_features.is_empty(),
        "crate features must not leak into non-Rust sessions"
    );
}

/// An audit run that never opened a documentation tree must say so. Reporting a bare
/// "Audit clean" for a run with no `--docs` root is what let a consumer's CI report green
/// while the documentation fence check never executed. ~keep
#[test]
fn an_audit_with_no_docs_root_names_the_check_class_it_skipped() {
    let summary = audit_scope_summary(&[]);
    assert!(
        summary.contains("NOT audited") && summary.contains("--docs"),
        "a docs-less audit must name the skipped scope and the flag that enables it; got: {summary}"
    );
}

/// Control: with a docs root configured the summary stays the plain clean line, so the
/// assertion above is discriminating rather than true of every message. ~keep
#[test]
fn an_audit_with_a_docs_root_reports_a_plain_clean_result() {
    let summary = audit_scope_summary(&[PathBuf::from("docs")]);
    assert_eq!(summary, "Audit clean: no issues found.");
}

/// `ExitCode` is neither `PartialEq` nor readable as an integer, so compare the one thing it
/// does expose. Stable within a process, which is all an in-process assertion needs. ~keep
fn is_success(code: ExitCode) -> bool {
    format!("{code:?}") == format!("{:?}", ExitCode::SUCCESS)
}

/// The consumer incident, reproduced: a snippet tree whose every file is vouched for by a
/// generated-coverage ledger, so the gap detector finds nothing to report even though it
/// opened no documentation page and compared no language.
///
/// Returns the snippet root. `languages` seeds one snippet file per named language directory,
/// each already recorded in the ledger. ~keep
fn ledger_backed_snippet_tree(root: &Path, languages: &[&str]) -> PathBuf {
    use crate::e2e::snippets::{
        COVERAGE_MANIFEST, COVERAGE_MANIFEST_VERSION, GeneratedSnippetMetadata, SnippetCoverageKey,
        SnippetCoverageLedger,
    };

    let snippets = root.join("snippets");
    let generated = snippets.join("generated");
    let mut generated_paths = Vec::new();
    let mut metadata = Vec::new();
    let mut keys = Vec::new();
    for language in languages {
        let relative = PathBuf::from(language).join("topic").join("generated.md");
        std::fs::create_dir_all(generated.join(language).join("topic")).expect("language directory");
        std::fs::write(generated.join(&relative), format!("```{language}\n// generated\n```\n"))
            .expect("generated snippet");
        let key = SnippetCoverageKey {
            fixture_id: "generated".into(),
            language: (*language).into(),
        };
        generated_paths.push(relative.clone());
        metadata.push(GeneratedSnippetMetadata {
            key: key.clone(),
            path: relative,
            language: (*language).into(),
            target: (*language).into(),
            session: (*language).into(),
            requires: Vec::new(),
            side_effect: crate::e2e::fixture::SideEffectClass::Safe,
        });
        keys.push(key);
    }
    let ledger = SnippetCoverageLedger {
        format_version: COVERAGE_MANIFEST_VERSION,
        generated_paths,
        generated_metadata: metadata,
        expected: keys.clone(),
        generated: keys,
        missing: Vec::new(),
        documented_exceptions: Vec::new(),
    };
    std::fs::write(
        generated.join(COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&ledger).expect("ledger serializes"),
    )
    .expect("coverage manifest");
    snippets
}

/// Half one: an unconfigured gap check must not be able to report a pass under `--strict`.
///
/// Without `--strict` it still exits zero — that is the documented behaviour for an
/// intentionally snippets-only invocation — which is exactly why the strict half has to exist:
/// a CI job whose entire purpose is gap detection would otherwise go green having compared
/// nothing. ~keep
#[test]
fn an_unconfigured_gap_check_passes_only_until_strict_is_asked_for() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = ledger_backed_snippet_tree(directory.path(), &["python"]);

    let lenient = run_gaps(&GapInvocation {
        snippet_dirs: std::slice::from_ref(&snippets),
        docs_dirs: &[],
        required_languages: None,
        include_base_paths: &[],
        strict: false,
    });
    assert!(
        is_success(lenient),
        "the tree really is gap-free by the detector's own reckoning; without --strict that stays a pass"
    );

    let strict = run_gaps(&GapInvocation {
        snippet_dirs: std::slice::from_ref(&snippets),
        docs_dirs: &[],
        required_languages: None,
        include_base_paths: &[],
        strict: true,
    });
    assert!(
        !is_success(strict),
        "--strict must refuse a verdict reached with no docs root and no required languages"
    );
}

/// Half two, the half that keeps half one from being vacuous: a *configured* run still finds a
/// real gap. A `--strict` flag that failed everything would satisfy the test above on its own.
/// ~keep
#[test]
fn a_configured_gap_check_still_detects_a_real_missing_language_variant() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = ledger_backed_snippet_tree(directory.path(), &["python"]);
    let docs = directory.path().join("docs");
    std::fs::create_dir_all(&docs).expect("docs root");
    std::fs::write(docs.join("index.md"), "# Usage\n").expect("docs page");

    let required = ["python".to_string(), "rust".to_string()];
    let code = run_gaps(&GapInvocation {
        snippet_dirs: std::slice::from_ref(&snippets),
        docs_dirs: std::slice::from_ref(&docs),
        required_languages: Some(&required),
        include_base_paths: std::slice::from_ref(&docs),
        strict: false,
    });

    assert!(
        !is_success(code),
        "the tree has a python snippet group and no rust variant; that is a gap and must fail even \
         without --strict"
    );
}

/// The other half of the sabotage check: a fully configured run over a tree with no gap must
/// still pass under `--strict`. Without this, `--strict` could be a blanket failure and both
/// tests above would still be green. ~keep
#[test]
fn a_configured_strict_gap_check_passes_when_there_is_no_gap() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = ledger_backed_snippet_tree(directory.path(), &["python", "rust"]);
    let docs = directory.path().join("docs");
    std::fs::create_dir_all(&docs).expect("docs root");
    std::fs::write(docs.join("index.md"), "# Usage\n").expect("docs page");

    let required = ["python".to_string(), "rust".to_string()];
    let code = run_gaps(&GapInvocation {
        snippet_dirs: std::slice::from_ref(&snippets),
        docs_dirs: std::slice::from_ref(&docs),
        required_languages: Some(&required),
        include_base_paths: std::slice::from_ref(&docs),
        strict: true,
    });

    assert!(
        is_success(code),
        "every required language is present and every snippet is ledger-backed; --strict must not \
         fail a run that genuinely compared something and found nothing"
    );
}
