//! Unit tests for [`super`], the documentation snippet gap detector.
//!
//! Split out of `gaps.rs` so that file drops under the 1,000-line cap this repository sets for
//! sources; the module keeps `use super::*` and is otherwise unchanged. ~keep

use super::*;
use crate::e2e::fixture::SideEffectClass;
use crate::e2e::snippets::{
    COVERAGE_MANIFEST_VERSION, GeneratedSnippetMetadata, MissingSnippet, SnippetCoverageKey, SnippetCoverageLedger,
};

#[test]
fn discovers_mkdocs_include_references() {
    let refs = parse_includes(
        r#"
--8<-- "snippets/python/example.md"
"#,
        Path::new("/repo/docs/index.md"),
        Path::new("/repo/docs"),
        &[],
    );

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].target, PathBuf::from("/repo/docs/snippets/python/example.md"));
    assert_eq!(refs[0].line, 2);
}

#[test]
fn reports_fixture_tree_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    let snippets = docs.join("snippets");
    std::fs::create_dir_all(snippets.join("python")).unwrap();
    std::fs::create_dir_all(snippets.join("rust")).unwrap();
    std::fs::write(docs.join("index.md"), r#"--8<-- "snippets/python/example.md""#).unwrap();
    std::fs::write(snippets.join("python/example.md"), "```python\nprint('ok')\n```\n").unwrap();
    std::fs::write(
        snippets.join("rust/unused.md"),
        "<!-- snippet:skip -->\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();

    let report = detect_gaps(&GapConfig {
        docs_dirs: vec![docs],
        snippet_dirs: vec![snippets],
        required_languages: vec![Language::Python, Language::Rust],
        include_base_paths: vec![],
        configured_references: vec![],
        exclude: vec![],
    })
    .unwrap();

    assert!(report.missing_references.is_empty());
    assert_eq!(report.unreferenced_snippets.len(), 1);
    assert_eq!(report.missing_language_variants.len(), 2);
    assert_eq!(report.skips_without_reason.len(), 1);
}

#[test]
fn generated_ledger_references_preserve_manual_orphan_detection() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = directory.path().join("snippets");
    let generated_root = snippets.join("generated");
    let generated = generated_root.join("python/topic/generated.md");
    let manual = snippets.join("python/topic/manual.md");
    std::fs::create_dir_all(generated.parent().expect("generated parent")).expect("snippet directory");
    std::fs::create_dir_all(manual.parent().expect("manual parent")).expect("manual snippet directory");
    std::fs::write(&generated, "```python\nvalue = 1\n```\n").expect("generated snippet");
    std::fs::write(&manual, "```python\nvalue = 2\n```\n").expect("manual snippet");
    std::fs::write(
        generated_root.join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&coverage_ledger(COVERAGE_MANIFEST_VERSION)).expect("coverage serializes"),
    )
    .expect("coverage manifest");

    let references = coverage_ledger_references(std::slice::from_ref(&snippets)).expect("current ledger");
    let report = detect_gaps(&GapConfig {
        snippet_dirs: vec![snippets],
        configured_references: references,
        ..GapConfig::default()
    })
    .expect("gap detection");

    assert_eq!(report.unreferenced_snippets, [manual]);
}

#[test]
fn nested_coverage_ledger_rejects_paths_outside_its_output_root() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = directory.path().join("snippets");
    let generated_root = snippets.join("generated");
    std::fs::create_dir_all(&generated_root).expect("generated snippet directory");
    std::fs::write(snippets.join("outside.md"), "```python\nvalue = 1\n```\n").expect("outside snippet");
    let mut ledger = coverage_ledger(COVERAGE_MANIFEST_VERSION);
    let outside = PathBuf::from("../outside.md");
    ledger.generated_paths[0] = outside.clone();
    ledger.generated_metadata[0].path = outside;
    std::fs::write(
        generated_root.join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&ledger).expect("coverage serializes"),
    )
    .expect("coverage manifest");

    let error = coverage_ledger_references(&[snippets]).expect_err("outside path must fail");

    assert!(error.to_string().contains("must stay beneath its output root"));
}

#[test]
fn stale_coverage_ledger_is_rejected() {
    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::write(
        directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&coverage_ledger(0)).expect("coverage serializes"),
    )
    .expect("coverage manifest");

    let error = coverage_ledger_references(&[directory.path().to_path_buf()]).expect_err("stale ledger must fail");

    assert!(error.to_string().contains("coverage manifest version 0 is unsupported"));
}

#[test]
fn incomplete_ledger_is_rejected_by_default_and_tolerated_on_request() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let generated = directory.path().join("python/topic/generated.md");
    std::fs::create_dir_all(generated.parent().expect("generated parent")).expect("snippet directory");
    std::fs::write(&generated, "```python\nvalue = 1\n```\n").expect("generated snippet");
    let mut ledger = coverage_ledger(COVERAGE_MANIFEST_VERSION);
    let absent = SnippetCoverageKey {
        fixture_id: "extension_only".into(),
        language: "python".into(),
    };
    ledger.expected.push(absent.clone());
    ledger.missing.push(MissingSnippet {
        key: absent,
        reason: "no compatible recipe".into(),
    });
    std::fs::write(
        directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&ledger).expect("coverage serializes"),
    )
    .expect("coverage manifest");
    let roots = [directory.path().to_path_buf()];

    let error = coverage_ledger_references(&roots).expect_err("incomplete ledger must fail by default");
    assert!(
        error
            .to_string()
            .contains("incomplete fixture-snippet coverage manifest")
    );

    let references =
        coverage_ledger_references_allowing_missing_cells(&roots).expect("missing cells are tolerated on request");
    assert_eq!(references, vec![generated]);
}

#[test]
fn tolerating_missing_cells_still_rejects_an_absent_generated_file() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let mut ledger = coverage_ledger(COVERAGE_MANIFEST_VERSION);
    let absent = SnippetCoverageKey {
        fixture_id: "extension_only".into(),
        language: "python".into(),
    };
    ledger.expected.push(absent.clone());
    ledger.missing.push(MissingSnippet {
        key: absent,
        reason: "no compatible recipe".into(),
    });
    std::fs::write(
        directory.path().join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&ledger).expect("coverage serializes"),
    )
    .expect("coverage manifest");

    let error = coverage_ledger_references_allowing_missing_cells(&[directory.path().to_path_buf()])
        .expect_err("a recorded generated file that is absent from disk must still fail");

    assert!(
        error
            .to_string()
            .contains("fixture snippet recorded by the coverage ledger is missing")
    );
}

fn coverage_ledger(format_version: u32) -> SnippetCoverageLedger {
    let key = SnippetCoverageKey {
        fixture_id: "generated".into(),
        language: "python".into(),
    };
    SnippetCoverageLedger {
        format_version,
        generated_paths: vec![PathBuf::from("python/topic/generated.md")],
        generated_metadata: vec![GeneratedSnippetMetadata {
            key: key.clone(),
            path: PathBuf::from("python/topic/generated.md"),
            language: "python".into(),
            target: "python".into(),
            session: "python".into(),
            requires: Vec::new(),
            side_effect: SideEffectClass::Safe,
        }],
        expected: vec![key.clone()],
        generated: vec![key],
        missing: Vec::new(),
        documented_exceptions: Vec::new(),
    }
}

/// Build a coverage ledger and matching snippet tree for three fixtures under `snippets_root`,
/// mirroring a real e2e-generated tree: `download` was dropped for `java` by
/// `exclude_functions` (never enters `expected`), `other` genuinely has both languages, and
/// `flaky` was expected to have both but `java` never got generated -- a real gap that must
/// keep failing. Returns the snippet root. ~keep
fn ledger_backed_tree_with_an_excluded_and_a_genuine_gap(snippets_root: &Path) -> PathBuf {
    let generated_root = snippets_root.join("generated");
    let mut generated_paths = Vec::new();
    let mut generated_metadata = Vec::new();
    let mut expected = Vec::new();
    let mut generated_keys = Vec::new();

    // (fixture, language, expected?, generated?)
    let cells = [
        ("download", "python", true, true),
        ("download", "java", false, false), // excluded via exclude_functions: never expected
        ("other", "python", true, true),
        ("other", "java", true, true),
        ("flaky", "python", true, true),
        ("flaky", "java", true, false), // expected, but generation genuinely never produced it
    ];
    for (fixture, language, is_expected, is_generated) in cells {
        let key = SnippetCoverageKey {
            fixture_id: fixture.into(),
            language: language.into(),
        };
        if is_expected {
            expected.push(key.clone());
        }
        if is_generated {
            let relative = PathBuf::from(language).join(fixture).join("generated.md");
            std::fs::create_dir_all(generated_root.join(language).join(fixture)).expect("fixture directory");
            std::fs::write(
                generated_root.join(&relative),
                format!("```{language}\n// {fixture}\n```\n"),
            )
            .expect("generated snippet");
            generated_paths.push(relative.clone());
            generated_metadata.push(GeneratedSnippetMetadata {
                key: key.clone(),
                path: relative,
                language: language.into(),
                target: language.into(),
                session: language.into(),
                requires: Vec::new(),
                side_effect: SideEffectClass::Safe,
            });
            generated_keys.push(key);
        }
    }
    let ledger = SnippetCoverageLedger {
        format_version: COVERAGE_MANIFEST_VERSION,
        generated_paths,
        generated_metadata,
        expected,
        generated: generated_keys,
        missing: Vec::new(),
        documented_exceptions: Vec::new(),
    };
    std::fs::write(
        generated_root.join(crate::e2e::snippets::COVERAGE_MANIFEST),
        serde_json::to_vec_pretty(&ledger).expect("coverage serializes"),
    )
    .expect("coverage manifest");
    generated_root
}

/// The consumer incident: `exclude_functions` dropped `download` for `java`, so `java` never
/// enters that fixture's `expected` set. The gap pass must not report that as a missing
/// language variant -- it never existed to be missing. ~keep
#[test]
fn a_function_dropped_by_exclude_functions_is_not_reported_as_a_missing_language_variant() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = ledger_backed_tree_with_an_excluded_and_a_genuine_gap(directory.path());

    let report = detect_gaps(&GapConfig {
        snippet_dirs: vec![snippets],
        required_languages: vec![Language::Python, Language::Java],
        ..GapConfig::default()
    })
    .expect("gap detection");

    assert!(
        !report
            .missing_language_variants
            .iter()
            .any(|variant| variant.language == Language::Java && variant.group.ends_with("download/generated.md")),
        "download's java variant was excluded via exclude_functions, not missing: {:?}",
        report.missing_language_variants
    );
}

/// The other half of the sabotage check for the fix above: a fixture the ledger genuinely
/// expected in both languages, but only generated one of, must still be reported. Suppressing
/// every ledger-tracked absence -- not just the ones `expected` actually excludes -- would
/// satisfy the test above for the wrong reason. ~keep
#[test]
fn a_language_the_ledger_expected_but_never_generated_still_reports_a_gap() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let snippets = ledger_backed_tree_with_an_excluded_and_a_genuine_gap(directory.path());

    let report = detect_gaps(&GapConfig {
        snippet_dirs: vec![snippets],
        required_languages: vec![Language::Python, Language::Java],
        ..GapConfig::default()
    })
    .expect("gap detection");

    assert!(
        report
            .missing_language_variants
            .iter()
            .any(|variant| variant.language == Language::Java && variant.group.ends_with("flaky/generated.md")),
        "flaky was expected to have a java variant and never generated one -- that is a real gap: {:?}",
        report.missing_language_variants
    );
}

#[test]
fn resolves_changelog_include_via_project_root_base_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let docs = root.join("docs");
    let snippets = docs.join("snippets");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(&snippets).unwrap();
    std::fs::write(root.join("CHANGELOG.md"), "# Changelog\n").unwrap();
    std::fs::write(docs.join("changelog.md"), r#"--8<-- "CHANGELOG.md""#).unwrap();

    let report = detect_gaps(&GapConfig {
        docs_dirs: vec![docs],
        snippet_dirs: vec![snippets],
        required_languages: vec![],
        include_base_paths: vec![root.to_path_buf()],
        configured_references: vec![],
        exclude: vec![],
    })
    .unwrap();

    assert!(
        report.missing_references.is_empty(),
        "expected no missing references, got: {:?}",
        report.missing_references
    );
}

#[test]
fn parses_mdx_content_import() {
    let target = parse_mdx_content_import_target(
        r#"import { Content as Snip_cli_install_cargo } from "../../../snippets/cli/install_cargo.md";"#,
    );
    assert_eq!(target, Some("../../../snippets/cli/install_cargo.md"));
}

#[test]
fn parses_bare_content_import_without_alias() {
    let target = parse_mdx_content_import_target(r#"import { Content } from "../snippets/x.md";"#);
    assert_eq!(target, Some("../snippets/x.md"));
}

#[test]
fn ignores_non_content_imports() {
    assert_eq!(
        parse_mdx_content_import_target(r#"import { Card } from "@astrojs/starlight/components";"#),
        None
    );
    assert_eq!(
        parse_mdx_content_import_target(r#"import Layout from "../Layout.astro";"#),
        None
    );
}

#[test]
fn parses_multiple_mdx_content_imports_in_one_file() {
    let content = concat!(
        "---\ntitle: Usage\n---\n",
        "import { Content as Snip_a } from \"../../../snippets/cli/a.md\";\n",
        "import { Content as Snip_b } from \"../../../snippets/cli/b.md\";\n",
    );
    let refs = parse_mdx_content_imports(content, Path::new("/repo/a/b/c/usage.mdx"));
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].target, PathBuf::from("/repo/snippets/cli/a.md"));
    assert_eq!(refs[0].line, 4);
    assert_eq!(refs[1].target, PathBuf::from("/repo/snippets/cli/b.md"));
    assert_eq!(refs[1].line, 5);
}

#[test]
fn mdx_import_path_is_relative_to_importing_file_not_docs_dir() {
    // The importing file lives three directories below `docs_dir` (mirroring
    // the real `docs/<section>/<page>.mdx` -> `../../../snippets/...` shape),
    // so resolving against `docs_dir` directly (the MkDocs behavior) would
    // produce the wrong path. Resolution must instead be relative to the
    // importing file's own directory. ~keep
    let refs = parse_mdx_content_imports(
        r#"import { Content as Snip_x } from "../../../snippets/cli/install_cargo.md";"#,
        Path::new("/repo/docs-site/src/content/docs/cli/usage.mdx"),
    );
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].target,
        PathBuf::from("/repo/docs-site/src/snippets/cli/install_cargo.md")
    );
}

#[test]
fn walks_mdx_files_for_references() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("content").join("docs");
    let snippets = dir.path().join("content").join("snippets");
    std::fs::create_dir_all(docs.join("cli")).unwrap();
    std::fs::create_dir_all(snippets.join("cli")).unwrap();
    std::fs::write(
        docs.join("cli").join("usage.mdx"),
        "import { Content as Snip_x } from \"../../snippets/cli/install_cargo.md\";\n",
    )
    .unwrap();
    std::fs::write(
        snippets.join("cli").join("install_cargo.md"),
        "```bash\ncargo install alef\n```\n",
    )
    .unwrap();
    std::fs::write(snippets.join("cli").join("orphan.md"), "```bash\necho orphan\n```\n").unwrap();

    let report = detect_gaps(&GapConfig {
        docs_dirs: vec![docs],
        snippet_dirs: vec![snippets],
        required_languages: vec![],
        include_base_paths: vec![],
        configured_references: vec![],
        exclude: vec![],
    })
    .unwrap();

    assert_eq!(
        report.unreferenced_snippets.len(),
        1,
        "expected only orphan.md to be unreferenced, got: {:?}",
        report.unreferenced_snippets
    );
    assert!(report.unreferenced_snippets[0].ends_with("orphan.md"));
}

#[test]
fn walks_astro_files_for_references() {
    let directory = tempfile::tempdir().expect("temporary docs directory");
    let docs = directory.path().join("docs");
    let snippets = directory.path().join("snippets");
    std::fs::create_dir_all(&docs).expect("create docs directory");
    std::fs::create_dir_all(&snippets).expect("create snippets directory");
    std::fs::write(
        docs.join("example.astro"),
        "import { Content as Example } from \"../snippets/example.md\";\n",
    )
    .expect("write Astro page");
    std::fs::write(snippets.join("example.md"), "```rust\nlet value = 1;\n```\n").expect("write snippet");

    let references = discover_includes(&[docs], &[]).expect("discover Astro imports");

    assert_eq!(references.len(), 1);
    assert_eq!(references[0].target, snippets.join("example.md"));
}

#[test]
fn astro_collection_query_references_every_file_in_its_mapped_root() {
    let directory = tempfile::tempdir().expect("temporary docs directory");
    let docs = directory.path().join("docs");
    let snippets = directory.path().join("snippets-generated");
    std::fs::create_dir_all(&docs).expect("create docs directory");
    std::fs::create_dir_all(snippets.join("python")).expect("create snippets directory");
    std::fs::write(
        docs.join("Example.astro"),
        r#"const examples = await getCollection("apiExamples");"#,
    )
    .expect("write Astro component");
    let first = snippets.join("python/first.md");
    let second = snippets.join("python/second.md");
    std::fs::write(&first, "```python\nprint('first')\n```\n").expect("write first snippet");
    std::fs::write(&second, "```python\nprint('second')\n```\n").expect("write second snippet");
    let collections = BTreeMap::from([("apiExamples".to_string(), snippets)]);

    let references = astro_collection_references(&[docs], &collections).expect("discover collection references");

    assert_eq!(references, vec![first, second]);
}

#[test]
fn ignores_configured_astro_collection_until_docs_query_it() {
    let directory = tempfile::tempdir().expect("temporary docs directory");
    let docs = directory.path().join("docs");
    let snippets = directory.path().join("snippets-generated");
    std::fs::create_dir_all(&docs).expect("create docs directory");
    std::fs::create_dir_all(&snippets).expect("create snippets directory");
    std::fs::write(docs.join("Example.astro"), "const examples = [];\n").expect("write Astro component");
    std::fs::write(snippets.join("unused.md"), "```rust\nlet value = 1;\n```\n").expect("write snippet");
    let collections = BTreeMap::from([("apiExamples".to_string(), snippets)]);

    let references = astro_collection_references(&[docs], &collections).expect("scan docs");

    assert!(references.is_empty());
}

#[test]
fn readme_only_references_honor_language_redirects_and_prefixes() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = dir.path().join("docs-site/src/snippets");
    std::fs::create_dir_all(snippets.join("c/api")).unwrap();
    let snippet = snippets.join("c/api/client.md");
    std::fs::write(&snippet, "```c\nint client(void);\n```\n").unwrap();
    let readme: crate::core::config::ReadmeConfig = serde_json::from_value(serde_json::json!({
        "template_dir": null,
        "snippets_dir": "docs-site/src/snippets",
        "config": null,
        "output_pattern": null,
        "discord_url": null,
        "banner_url": null,
        "languages": {
            "ffi": {
                "snippet_language": "c",
                "snippets": { "client": "ffi/api/client.md" }
            }
        },
        "targets": {}
    }))
    .unwrap();
    let references = readme_snippet_references(dir.path(), Some(&readme));
    assert_eq!(references, vec![snippet.clone()]);

    let report = detect_gaps(&GapConfig {
        snippet_dirs: vec![snippets],
        configured_references: references,
        ..GapConfig::default()
    })
    .unwrap();
    assert!(report.unreferenced_snippets.is_empty());
    assert!(report.missing_references.is_empty());
}

#[test]
fn readme_references_honor_per_language_snippet_roots() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let manual = directory.path().join("manual");
    let generated = directory.path().join("generated");
    std::fs::create_dir_all(manual.join("python")).expect("manual root");
    std::fs::create_dir_all(generated.join("python")).expect("generated root");
    let generated_snippet = generated.join("python/quick_start.md");
    std::fs::write(&generated_snippet, "```python\nprint('ready')\n```\n").expect("generated snippet");
    let readme: crate::core::config::ReadmeConfig = serde_json::from_value(serde_json::json!({
        "snippets_dir": "manual",
        "languages": {
            "python": {
                "snippets_dir": "generated",
                "snippets": { "quick_start": "quick_start.md" }
            }
        }
    }))
    .expect("README config");

    let references = readme_snippet_references(directory.path(), Some(&readme));

    assert_eq!(references, vec![generated_snippet]);
}

#[test]
fn readme_references_honor_per_mapping_roots() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let readme: crate::core::config::ReadmeConfig = serde_json::from_value(serde_json::json!({
        "snippets_dir": "manual",
        "languages": {
            "python": {
                "snippets": {
                    "legacy": "legacy.md",
                    "current": {"path": "current.md", "root": "generated"}
                }
            }
        }
    }))
    .expect("README config");

    let references = readme_snippet_references(directory.path(), Some(&readme));

    assert_eq!(
        references,
        vec![
            directory.path().join("generated/python/current.md"),
            directory.path().join("manual/python/legacy.md"),
        ]
    );
}
