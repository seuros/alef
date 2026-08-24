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

#[test]
fn strict_coverage_rejects_every_non_validation_status() {
    assert!(is_incomplete_status(SnippetStatus::Skip));
    assert!(is_incomplete_status(SnippetStatus::Unavailable));
    assert!(is_incomplete_status(SnippetStatus::Downgraded));
    assert!(!is_incomplete_status(SnippetStatus::Pass));
}

#[test]
fn configured_audit_is_skipped_without_a_docs_surface() {
    let directory = tempfile::tempdir().expect("temp directory");
    let snippets = directory.path().join("snippets");
    std::fs::create_dir_all(&snippets).expect("snippet directory");
    std::fs::write(snippets.join("weird.md"), "```gibberish\nvalue\n```\n").expect("write snippet");
    let snippet_directories = [snippets];

    let (audit_failure, gap_failure) = run_configured_audit_and_gaps(&ConfiguredCheckInputs {
        snippet_directories: &snippet_directories,
        docs_directories: &[],
        include_base_paths: &[],
        required_languages: &[],
        exclude: &[],
        readme: None,
        content_collections: &std::collections::BTreeMap::new(),
        workspace_root: directory.path(),
        require_frontmatter: false,
        strict: true,
    })
    .expect("audit and gap pass");

    assert!(
        !audit_failure,
        "a snippets-only config has no documentation surface to audit, so an unknown fence tag \
         must not fail the gate — `docs/mod.rs::validate_snippets` skips audit the same way"
    );
    assert!(
        !gap_failure,
        "gaps are meaningless without docs dirs or required languages"
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
