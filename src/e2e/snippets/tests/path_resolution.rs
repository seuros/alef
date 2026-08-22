//! Snippet path resolution, docs-path validation, and language-alias tests, split
//! out of [`super`] (`mod.rs`), which the 1,000-line file-size cap no longer let
//! hold this concern inline.

use super::*;

#[test]
fn snippet_paths_reject_traversal() {
    let mut docs = FixtureDocs {
        topic: "..".into(),
        stem: None,
        paths: BTreeMap::new(),
        title: None,
        description: None,
        input: None,
        shows: Vec::new(),
        error: None,
        presentation: None,
        client: None,
        side_effects: Default::default(),
        coverage_exceptions: BTreeMap::new(),
    };
    assert!(
        snippet_path(
            "docs/snippets",
            &docs,
            "basic",
            "python",
            DocumentationLanguage::Binding(Language::Python)
        )
        .is_err()
    );
    docs.topic = "fallback".into();
    docs.paths.insert("python".into(), "../escape.md".into());
    assert!(
        snippet_path(
            "docs/snippets",
            &docs,
            "basic",
            "python",
            DocumentationLanguage::Binding(Language::Python)
        )
        .is_err()
    );
}

#[test]
fn target_path_override_precedes_topic_and_stem() {
    let docs = FixtureDocs {
        topic: "fallback".into(),
        stem: Some("fallback".into()),
        paths: BTreeMap::from([("node".into(), "config/basic_usage.md".into())]),
        title: None,
        description: None,
        input: None,
        shows: Vec::new(),
        error: None,
        presentation: None,
        client: None,
        side_effects: Default::default(),
        coverage_exceptions: BTreeMap::new(),
    };

    assert_eq!(
        snippet_path(
            "docs/snippets",
            &docs,
            "fixture",
            "node",
            DocumentationLanguage::Binding(Language::Node)
        )
        .expect("safe target path"),
        Path::new("docs/snippets/typescript/config/basic_usage.md")
    );
}

#[test]
fn docs_path_target_must_be_configured() {
    let mut fixture = documented_fixture();
    fixture
        .docs
        .as_mut()
        .expect("fixture docs")
        .paths
        .insert("wasm".into(), "browser/basic.md".into());

    assert!(validate_docs_paths(&fixture, &["node".into()]).is_err());
    assert!(validate_docs_paths(&fixture, &["node".into(), "wasm".into()]).is_ok());
}

#[test]
fn language_aliases_include_core_and_ffi_targets() {
    assert_eq!(
        parse_language("rust_core"),
        Some(DocumentationLanguage::Binding(Language::Rust))
    );
    assert_eq!(parse_language("ffi"), Some(DocumentationLanguage::Binding(Language::C)));
    assert_eq!(parse_language("brew"), Some(DocumentationLanguage::Shell));
    assert_eq!(parse_language("homebrew"), Some(DocumentationLanguage::Shell));
    assert_eq!(generator_name("rust_core"), "rust");
    assert_eq!(generator_name("ffi"), "c");
}

#[test]
fn generated_docs_use_validator_canonical_language_identity() {
    let docs = FixtureDocs {
        topic: "api".into(),
        stem: None,
        paths: BTreeMap::new(),
        title: None,
        description: None,
        input: None,
        shows: Vec::new(),
        error: None,
        presentation: None,
        client: None,
        side_effects: SideEffectClass::Safe,
        coverage_exceptions: BTreeMap::new(),
    };
    let cases = [
        ("node", Language::Node, "typescript", "typescript"),
        ("wasm", Language::Wasm, "typescript", "wasm"),
        ("kotlin_android", Language::KotlinAndroid, "kotlin", "kotlin-android"),
    ];

    for (target_language, binding_language, canonical_name, output_slug) in cases {
        let language = DocumentationLanguage::Binding(binding_language);
        let fixture = documented_fixture();
        let rendered = render_snippet_markdown("example()", &fixture, &docs, target_language, language);
        let path =
            snippet_path("docs/snippets", &docs, "example", target_language, language).expect("snippet path is valid");

        // ~keep Assert the WHOLE document, not a set of `contains` probes. Substring probes
        // pin only the fields they name, so `level`, `requires` and `side_effect` become
        // unguarded: a renderer emitting a bogus value for any of them still passes. See
        // `frontmatter_fields_are_pinned_by_exact_equality` for the controls.
        assert_eq!(
            rendered,
            format!(
                "---\nid: fixture_{target_language}_extension_owned\nlanguage: {canonical_name}\ntarget: {target_language}\nrequires: []\nside_effect: safe\n---\n\n{SNIPPET_HEADER}Extension-owned example\n\n```{canonical_name} title=\"{}\"\nexample()\n```\n",
                language.display_name()
            )
        );
        assert_eq!(
            path,
            Path::new("docs/snippets").join(output_slug).join("api/example.md")
        );
        assert_ne!(
            crate::snippets::types::Language::from_fence_tag(canonical_name),
            crate::snippets::types::Language::Unknown
        );
    }
}
