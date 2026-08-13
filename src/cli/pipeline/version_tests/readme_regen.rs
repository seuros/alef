use super::*;
use crate::core::config::{Language, ReadmeConfig, ResolvedCrateConfig};
use std::collections::HashMap;
use std::path::PathBuf;

fn readme_config(root: &std::path::Path, template_dir: PathBuf) -> ResolvedCrateConfig {
    let mut languages = HashMap::new();
    languages.insert(
        "rust".to_string(),
        serde_json::json!({
            "template": "rust.md",
            "output_path": "README.md"
        }),
    );
    ResolvedCrateConfig {
        name: "sample_core".to_string(),
        version_from: root.join("Cargo.toml").to_string_lossy().into_owned(),
        sources: vec![root.join("src/lib.rs")],
        workspace_root: Some(root.to_path_buf()),
        languages: vec![Language::Rust],
        readme: Some(ReadmeConfig {
            template_dir: Some(template_dir),
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages,
            targets: HashMap::new(),
        }),
        ..ResolvedCrateConfig::default()
    }
}

#[test]
fn sync_versions_without_regen_preserves_readme_content() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let original_cwd = std::env::current_dir().expect("current directory");
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    let templates = root.join("templates");
    std::fs::create_dir_all(root.join("src")).expect("source directory");
    std::fs::create_dir_all(&templates).expect("template directory");
    std::fs::write(root.join("src/lib.rs"), "pub fn transform() {}\n").expect("Rust source");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"sample_core\"\nversion = \"1.2.3\"\nedition = \"2024\"\n",
    )
    .expect("Cargo.toml");
    std::fs::write(root.join("package.json"), r#"{"name":"sample","version":"1.0.0"}"#).expect("package.json");
    std::fs::write(templates.join("rust.md"), "# Regenerated README\n").expect("README template");
    let original_readme = "# Hand-written Quick Start\n\nThis API shape is consumer-owned.\n";
    std::fs::write(root.join("README.md"), original_readme).expect("README");
    let config_path = root.join("alef.toml");
    std::fs::write(&config_path, "").expect("alef.toml");
    let config = readme_config(root, templates);

    std::env::set_current_dir(root).expect("enter temporary directory");
    let result = sync_versions(&config, &config_path, None, true, true, None);
    std::env::set_current_dir(&original_cwd).expect("restore current directory");
    result.expect("sync versions");

    assert_eq!(
        std::fs::read_to_string(root.join("README.md")).expect("read README"),
        original_readme
    );
    assert!(
        std::fs::read_to_string(root.join("package.json"))
            .expect("read package.json")
            .contains("1.2.3"),
        "version-coordinate edits remain active without regeneration"
    );

    std::fs::write(root.join("package.json"), r#"{"name":"sample","version":"1.0.0"}"#).expect("reset package.json");
    std::env::set_current_dir(root).expect("re-enter temporary directory");
    let result = sync_versions(&config, &config_path, None, false, true, None);
    std::env::set_current_dir(&original_cwd).expect("restore current directory");
    result.expect("sync versions with regeneration");
    assert_eq!(
        std::fs::read_to_string(root.join("README.md")).expect("read regenerated README"),
        "# Regenerated README\n"
    );
}
