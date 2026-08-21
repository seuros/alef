use super::*;
use crate::core::config::ReadmeConfig;
use std::collections::HashMap;
use std::fs;
use tracing_test::traced_test;

// --- heading_level ---

#[test]
fn should_detect_atx_headings_at_every_level() {
    for (line, expected) in [
        ("# Title", Some(1)),
        ("###### Deepest", Some(6)),
        ("## Trailing space handling ", Some(2)),
        ("###NoSpace", None),
        ("Not a heading", None),
        ("#######SevenHashes", None),
    ] {
        assert_eq!(heading_level(line), expected, "line: {line:?}");
    }
}

#[test]
fn should_treat_a_bare_hash_line_as_a_heading() {
    assert_eq!(heading_level("#"), Some(1));
}

// --- find_empty_headings ---

#[test]
fn should_flag_a_heading_immediately_followed_by_a_same_level_heading() {
    let content = "### Common Use Cases\n\n### Next Steps\n\nSome text.\n";
    assert_eq!(find_empty_headings(content), vec!["### Common Use Cases".to_string()]);
}

#[test]
fn should_flag_a_heading_immediately_followed_by_a_shallower_heading() {
    let content = "### Empty\n\n## Next Section\n\nSome text.\n";
    assert_eq!(find_empty_headings(content), vec!["### Empty".to_string()]);
}

#[test]
fn should_flag_the_last_heading_when_nothing_follows_it() {
    let content = "# Title\n\nBody.\n\n## Dangling\n\n";
    assert_eq!(find_empty_headings(content), vec!["## Dangling".to_string()]);
}

#[test]
fn should_not_flag_a_heading_grouping_a_deeper_subsection() {
    // `## Examples` has no prose of its own -- it groups `### Streaming Responses` -- and
    // that is a legitimate, common README shape, not an empty section.
    let content = "## Examples\n\n### Streaming Responses\n\nStream tokens.\n";
    assert!(find_empty_headings(content).is_empty());
}

#[test]
fn should_not_flag_a_heading_followed_by_prose_or_a_list() {
    let content = "## Documentation\n\n- [Docs](https://example.com)\n";
    assert!(find_empty_headings(content).is_empty());
}

#[test]
fn should_ignore_hash_comments_inside_fenced_code_blocks() {
    // Shell/Python/C comments and `#include` directives inside fenced examples must never be
    // misread as Markdown structure, and the fence itself is body content.
    let content =
        "### Package Installation\n\n```bash\n# comment, not a heading\npip install my-lib\n```\n\n### Next\n\nMore.\n";
    assert!(find_empty_headings(content).is_empty());
}

#[test]
fn should_flag_a_heading_whose_only_content_is_blank_lines() {
    let content = "### Common Use Cases\n\n\n### Next Steps\n\nBody.\n";
    assert_eq!(find_empty_headings(content), vec!["### Common Use Cases".to_string()]);
}

// --- end-to-end: generate_readmes warns, but still ships, an empty-body heading ---
//
// This is a docs-tidiness signal (a dangling TOC entry), not a correctness break, and alef is
// consumed by several repos mid-release -- a template shape this scanner misjudges, or a
// section a consumer deliberately leaves empty, must not turn into an unrelated hard build
// failure. It must still be visible, though, so it is a `tracing::warn!`, not silent.

#[traced_test]
#[test]
fn generate_readmes_warns_but_succeeds_when_a_rendered_readme_has_an_empty_heading() {
    let tmp = std::env::temp_dir().join("alef_readme_empty_heading_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::write(
        tmp.join("test.md"),
        "# {{ name }}\n\n### Common Use Cases\n\n### Next Steps\n\n- Learn more.\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(tmp.clone());
    let mut languages = HashMap::new();
    languages.insert(
        "python".to_string(),
        serde_json::json!({ "template": "test.md", "output_path": "packages/python/README.md" }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages,
        targets: HashMap::new(),
    });

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python])
        .expect("an empty-body heading must warn, not fail, generation");
    assert!(
        files[0].content.contains("### Common Use Cases"),
        "the heading must still ship -- this is a warning, not a fix -- got: {}",
        files[0].content
    );
    assert!(
        logs_contain("Common Use Cases"),
        "the offending heading must be named in a warning"
    );

    let _ = fs::remove_dir_all(&tmp);
}
