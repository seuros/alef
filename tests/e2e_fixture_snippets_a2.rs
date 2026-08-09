use alef::core::backend::GeneratedFile;
use alef::core::config::e2e::E2eConfig;
use alef::e2e::fixture::{Fixture, SideEffectClass};
use alef::e2e::snippets::migration::{MigrationStatus, compare_existing};
use std::path::{Path, PathBuf};

#[test]
fn fixture_docs_metadata_and_requirements_deserialize() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "basic",
        "description": "Convert a value",
        "docs": { "topic": "convert", "stem": "basic", "side_effects": "network" },
        "requirements": ["feature:json", "service:example"]
    }))
    .expect("fixture metadata should deserialize");

    let docs = fixture.docs.expect("docs metadata should be present");
    assert_eq!(docs.topic, "convert");
    assert_eq!(docs.side_effects, SideEffectClass::Network);
    assert_eq!(fixture.requirements, ["feature:json", "service:example"]);
}

#[test]
fn snippet_config_is_optional_and_deterministic() {
    let config: E2eConfig = toml::from_str(
        r#"
        [call]
        function = "convert"

        [snippets]
        output = "docs/snippets"

        [snippets.capabilities]
        all = ["service:example"]
        python = ["model:small"]
        "#,
    )
    .expect("snippet config should deserialize");

    let snippets = config.snippets.expect("snippet config should be present");
    assert_eq!(snippets.output, "docs/snippets");
    assert_eq!(
        snippets
            .capabilities
            .for_language("python")
            .into_iter()
            .collect::<Vec<_>>(),
        ["model:small", "service:example"]
    );
}

#[test]
fn migration_comparison_distinguishes_all_statuses() {
    let generated = vec![
        GeneratedFile {
            path: PathBuf::from("python/topic/same.md"),
            content: "same".into(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("python/topic/changed.md"),
            content: "new".into(),
            generated_header: false,
        },
    ];
    let report = compare_existing(
        [
            (Path::new("python/topic/same.md"), "same"),
            (Path::new("python/topic/changed.md"), "old"),
            (Path::new("python/topic/manual.md"), "manual"),
        ],
        &generated,
    );

    assert_eq!(report[0].status, MigrationStatus::Identical);
    assert_eq!(report[1].status, MigrationStatus::Different);
    assert_eq!(report[2].status, MigrationStatus::NoGeneratedEquivalent);
}
