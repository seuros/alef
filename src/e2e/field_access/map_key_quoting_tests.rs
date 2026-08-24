//! A string map key written with quotes in a config path (`labels["theme"]`) must reach the
//! target language quoted exactly once. `parse_path` used to carry the quotes into the key, and
//! every renderer then added its own: Swift/Go/Java/Ruby/... emitted the unparseable
//! `labels[""theme""]`, while TypeScript emitted `labels["\"theme\""]` — valid TypeScript that
//! silently looks up a key no map holds.

use super::*;
use std::collections::{HashMap, HashSet};

/// `(language, accessor)` for every language `render_accessor` and the optional-aware renderers
/// dispatch on, for the map path `labels[<key>]`.
fn accessors_for(path: &str) -> Vec<(&'static str, String)> {
    const LANGUAGES: &[&str] = &[
        "rust",
        "python",
        "typescript",
        "node",
        "wasm",
        "go",
        "java",
        "kotlin",
        "kotlin_android",
        "csharp",
        "ruby",
        "php",
        "elixir",
        "r",
        "c",
        "swift",
        "dart",
        "zig",
        "gleam",
    ];
    let mut fields = HashMap::new();
    fields.insert("theme".to_string(), path.to_string());
    let resolver = FieldResolver::new(
        &fields,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    LANGUAGES
        .iter()
        .map(|language| (*language, resolver.accessor("theme", language, "result")))
        .collect()
}

/// The accessor every language must produce for the map key `theme`, however the config path
/// spelled it. Each entry is quoted exactly once and names the key `theme`, not `"theme"`.
fn expected_singly_quoted_theme() -> Vec<(&'static str, &'static str)> {
    vec![
        ("rust", r#"result.labels.get("theme").map(|s| s.as_str())"#),
        ("python", r#"result.labels.get("theme")"#),
        ("typescript", r#"result.labels["theme"]"#),
        ("node", r#"result.labels["theme"]"#),
        ("wasm", r#"result.labels.get("theme")"#),
        ("go", r#"result.Labels["theme"]"#),
        ("java", r#"result.labels().get("theme")"#),
        ("kotlin", r#"result.labels().get("theme")"#),
        ("kotlin_android", r#"result.labels.get("theme")"#),
        ("csharp", r#"result.Labels["theme"]"#),
        ("ruby", r#"result.labels["theme"]"#),
        ("php", r#"result->labels["theme"]"#),
        ("elixir", r#"result.labels["theme"]"#),
        ("r", r#"result$labels[["theme"]]"#),
        ("c", r#"result_labels(result)["theme"]"#),
        ("swift", r#"result.labels()["theme"]"#),
        ("dart", r#"result.labels["theme"]"#),
        ("zig", r#"result.labels.get("theme")"#),
        ("gleam", r#"result.labels.get("theme")"#),
    ]
}

fn assert_matches_expected(path: &str) {
    let actual = accessors_for(path);
    let expected = expected_singly_quoted_theme();
    assert_eq!(
        actual.len(),
        expected.len(),
        "language list and expectation table disagree"
    );
    for ((language, rendered), (expected_language, wanted)) in actual.iter().zip(expected.iter()) {
        assert_eq!(language, expected_language, "language tables out of order");
        assert_eq!(rendered, wanted, "path `{path}` rendered the wrong {language} accessor");
    }
}

#[test]
fn double_quoted_map_key_is_quoted_exactly_once_in_every_language() {
    assert_matches_expected(r#"labels["theme"]"#);
}

#[test]
fn single_quoted_map_key_is_quoted_exactly_once_in_every_language() {
    assert_matches_expected("labels['theme']");
}

#[test]
fn bare_map_key_renders_identically_to_a_quoted_one() {
    assert_matches_expected("labels[theme]");
}

#[test]
fn no_language_emits_a_doubled_or_escaped_quote_for_a_plain_key() {
    for (language, rendered) in accessors_for(r#"labels["theme"]"#) {
        assert!(
            !rendered.contains("\"\""),
            "{language} double-quoted the key: {rendered}"
        );
        assert!(
            !rendered.contains("\\\""),
            "{language} escaped a quote into the key: {rendered}"
        );
        assert!(
            rendered.contains("\"theme\""),
            "{language} lost the key `theme`: {rendered}"
        );
    }
}

#[test]
fn a_quoted_digit_key_indexes_like_a_bare_digit_key() {
    // Quotes are bracket syntax, not type information: `labels["0"]` and `labels[0]` are the
    // same index. Before the fix the quoted form fell through to string-map access and emitted
    // `labels[""0""]`.
    let quoted = accessors_for(r#"labels["0"]"#);
    let bare = accessors_for("labels[0]");
    assert_eq!(quoted, bare);
    let typescript = &quoted
        .iter()
        .find(|(language, _)| *language == "typescript")
        .expect("typescript accessor")
        .1;
    assert_eq!(typescript, "result.labels[0]");
}

/// Languages whose map-key literal is emitted by `renderers.rs` (plus TypeScript/node, whose
/// `optional_renderers` path already formats the key with `{:?}`). The remaining languages route
/// their map access through `optional_renderers.rs`, which still interpolates the key raw.
const ESCAPING_LANGUAGES: &[&str] = &[
    "typescript",
    "node",
    "wasm",
    "go",
    "ruby",
    "elixir",
    "python",
    "gleam",
    "swift",
];

fn accessor_for(path: &str, language: &str) -> String {
    accessors_for(path)
        .into_iter()
        .find(|(candidate, _)| candidate == &language)
        .unwrap_or_else(|| panic!("no accessor for {language}"))
        .1
}

#[test]
fn a_key_containing_a_quote_stays_inside_its_literal() {
    // `labels["a"b"]` in a config path yields the key `a"b` once the delimiters are stripped.
    // Interpolating it raw would close the literal early and emit code that does not parse.
    let expected = [
        ("typescript", r#"result.labels["a\"b"]"#),
        ("node", r#"result.labels["a\"b"]"#),
        ("wasm", r#"result.labels.get("a\"b")"#),
        ("go", r#"result.Labels["a\"b"]"#),
        ("ruby", r#"result.labels["a\"b"]"#),
        ("elixir", r#"result.labels["a\"b"]"#),
        ("python", r#"result.labels.get("a\"b")"#),
        ("gleam", r#"result.labels.get("a\"b")"#),
        ("swift", r#"result.labels()["a\"b"]"#),
    ];
    assert_eq!(expected.len(), ESCAPING_LANGUAGES.len());
    for (language, wanted) in expected {
        assert_eq!(accessor_for(r#"labels["a"b"]"#, language), wanted, "{language}");
    }
}

#[test]
fn a_key_containing_a_backslash_stays_inside_its_literal() {
    let expected = [
        ("typescript", r#"result.labels["a\\b"]"#),
        ("node", r#"result.labels["a\\b"]"#),
        ("wasm", r#"result.labels.get("a\\b")"#),
        ("go", r#"result.Labels["a\\b"]"#),
        ("ruby", r#"result.labels["a\\b"]"#),
        ("elixir", r#"result.labels["a\\b"]"#),
        ("python", r#"result.labels.get("a\\b")"#),
        ("gleam", r#"result.labels.get("a\\b")"#),
        ("swift", r#"result.labels()["a\\b"]"#),
    ];
    assert_eq!(expected.len(), ESCAPING_LANGUAGES.len());
    for (language, wanted) in expected {
        assert_eq!(accessor_for(r#"labels["a\b"]"#, language), wanted, "{language}");
    }
}

#[test]
fn quoted_map_key_survives_a_longer_path() {
    let mut fields = HashMap::new();
    fields.insert(
        "title".to_string(),
        r#"metadata.open_graph_tags["og_title"].value"#.to_string(),
    );
    let resolver = FieldResolver::new(
        &fields,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(
        resolver.accessor("title", "typescript", "result"),
        r#"result.metadata.openGraphTags["og_title"].value"#
    );
    assert_eq!(
        resolver.accessor("title", "go", "result"),
        r#"result.Metadata.OpenGraphTags["og_title"].Value"#
    );
}
