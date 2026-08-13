use super::*;
use std::fs;
use std::path::PathBuf;

#[test]
fn readme_language_mapping_can_override_the_shared_snippet_root() {
    let temporary = tempfile::tempdir().expect("temporary README workspace");
    let generated = temporary.path().join("generated/python");
    fs::create_dir_all(&generated).expect("generated snippet root");
    fs::create_dir_all(temporary.path().join("manual")).expect("manual snippet root");
    fs::write(generated.join("hello.py"), "print('generated')").expect("generated snippet");
    fs::write(
        temporary.path().join("template.md"),
        r#"{{ "hello.py" | include_snippet("python") }}"#,
    )
    .expect("README template");

    let mut config = test_config();
    config.readme = Some(ReadmeConfig {
        template_dir: Some(temporary.path().to_path_buf()),
        snippets_dir: Some(PathBuf::from("manual")),
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::from([(
            "python".to_string(),
            serde_json::json!({
                "template": "template.md",
                "output_path": "packages/python/README.md",
                "snippets_dir": "generated"
            }),
        )]),
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(temporary.path().to_path_buf());

    let files = generate_readmes(&test_api(), &config, &[Language::Python]).expect("generated README");

    assert!(files[0].content.contains("print('generated')"));
}
