use super::*;
use crate::core::config::{Language, ReadmeConfig};
use crate::core::ir::{FunctionDef, TypeDef};
use std::fs;

fn render_function_names(lang: Language, expected: &str) {
    let temporary = tempfile::tempdir().expect("temporary README workspace");
    fs::write(
        temporary.path().join("language.md"),
        concat!(
            "{% for function in functions %}",
            "{{ function.name }}|{{ function.rust_name }}|{{ function.is_async }}|",
            "{{ function.documentation }}\n",
            "{% endfor %}",
        ),
    )
    .expect("README template");

    let mut config = test_config();
    config.workspace_root = Some(temporary.path().to_path_buf());
    config.readme = Some(ReadmeConfig {
        template_dir: Some(temporary.path().to_path_buf()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::from([(
            crate::readme::paths::lang_code(lang).to_string(),
            serde_json::json!({ "template": "language.md" }),
        )]),
        targets: std::collections::HashMap::new(),
    });

    let mut api = test_api();
    api.functions = vec![FunctionDef {
        name: "parse_html_document".to_string(),
        rust_path: "sample::parse_html_document".to_string(),
        original_rust_path: "sample::parse_html_document".to_string(),
        is_async: true,
        doc: "Parse a document.".to_string(),
        ..FunctionDef::default()
    }];

    let files = generate_readmes(&api, &config, &[lang]).expect("render README");
    // README output now carries a self-embedded HTML-comment header (see
    // `template.rs`'s `~keep` note) ahead of the rendered template body, so
    // the rendered body is a suffix of the full content, not the whole thing.
    assert!(
        files[0].content.ends_with(expected),
        "expected content to end with {expected:?}, got:\n{}",
        files[0].content
    );
}

#[test]
fn readme_function_names_follow_each_language_public_naming_policy() {
    for (lang, expected) in [
        (
            Language::Python,
            "parse_html_document|parse_html_document|True|Parse a document.\n",
        ),
        (
            Language::Node,
            "parseHtmlDocument|parse_html_document|True|Parse a document.\n",
        ),
        (
            Language::Go,
            "ParseHTMLDocument|parse_html_document|True|Parse a document.\n",
        ),
        (
            Language::Csharp,
            "ParseHtmlDocument|parse_html_document|True|Parse a document.\n",
        ),
        (
            Language::Zig,
            "parse_html_document|parse_html_document|True|Parse a document.\n",
        ),
        (
            Language::Ffi,
            "my_lib_parse_html_document|parse_html_document|True|Parse a document.\n",
        ),
    ] {
        render_function_names(lang, expected);
    }
}

#[test]
fn readme_function_surface_matches_language_excludes_and_go_collisions() {
    let temporary = tempfile::tempdir().expect("temporary README workspace");
    fs::write(
        temporary.path().join("language.md"),
        "{% for function in functions %}{{ function.name }}{% if not loop.last %},{% endif %}{% endfor %}\n",
    )
    .expect("README template");

    let mut config = test_config();
    config.workspace_root = Some(temporary.path().to_path_buf());
    config.go = Some(toml::from_str("exclude_functions = ['hidden_call']").expect("Go config"));
    config.readme = Some(ReadmeConfig {
        template_dir: Some(temporary.path().to_path_buf()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::from([(
            "go".to_string(),
            serde_json::json!({ "template": "language.md" }),
        )]),
        targets: std::collections::HashMap::new(),
    });

    let mut api = test_api();
    api.types.push(TypeDef {
        name: "ModelInfo".to_string(),
        ..TypeDef::default()
    });
    api.functions = ["model_info", "visible_call", "hidden_call"]
        .into_iter()
        .map(|name| FunctionDef {
            name: name.to_string(),
            rust_path: format!("sample::{name}"),
            ..FunctionDef::default()
        })
        .collect();
    api.functions.push(FunctionDef {
        name: "binding_hidden".to_string(),
        rust_path: "sample::binding_hidden".to_string(),
        binding_excluded: true,
        ..FunctionDef::default()
    });
    api.functions.push(FunctionDef {
        name: "feature_hidden".to_string(),
        rust_path: "sample::feature_hidden".to_string(),
        cfg: Some("feature = \"not_enabled\"".to_string()),
        ..FunctionDef::default()
    });

    let files = generate_readmes(&api, &config, &[Language::Go]).expect("render README");
    assert!(
        files[0].content.ends_with("GetModelInfo,VisibleCall\n"),
        "got:\n{}",
        files[0].content
    );
}
