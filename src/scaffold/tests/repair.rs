//! Integration coverage for [`repair_missing_cfg_binding_features`] against the shape that
//! motivated it: a Ruby and an Elixir native manifest that already declare
//! `native-http`/`opendal-cache`/`wasm-http`, generated source that forwards `#[cfg(feature =
//! "tokenizer")]` / `#[cfg(feature = "tower")]` for three functions
//! (`count_tokens`/`count_request_tokens`/`record_cost_usd`), and a core crate that genuinely
//! declares both `tokenizer` and `tower`.

use super::*;
use crate::core::ir::FunctionDef;
use std::collections::BTreeSet;

const EXISTING_MANIFEST: &str = "[package]\nname = \"sample-core-rb\"\nversion = \"0.1.0\"\n\n[features]\n\
     default = [\"native-http\"]\n\
     native-http = [\"sample-core/native-http\"]\n\
     opendal-cache = [\"sample-core/opendal-cache\"]\n\
     wasm-http = [\"sample-core/wasm-http\"]\n\n\
     [dependencies]\nmagnus = \"0.7\"\n";

fn sample_config(ws_root: &std::path::Path) -> ResolvedCrateConfig {
    let mut config = test_config();
    config.workspace_root = Some(ws_root.to_path_buf());
    config.name = "sample-core".to_string();
    config.sources = vec![PathBuf::from("crates/sample-core/src/lib.rs")];
    config
}

fn sample_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample_core".to_string(),
        functions: vec![
            FunctionDef {
                name: "count_tokens".to_string(),
                rust_path: "sample_core::count_tokens".to_string(),
                cfg: Some(r#"feature = "tokenizer""#.to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "count_request_tokens".to_string(),
                rust_path: "sample_core::count_request_tokens".to_string(),
                cfg: Some(r#"feature = "tokenizer""#.to_string()),
                ..Default::default()
            },
            FunctionDef {
                name: "record_cost_usd".to_string(),
                rust_path: "sample_core::record_cost_usd".to_string(),
                cfg: Some(r#"feature = "tower""#.to_string()),
                ..Default::default()
            },
        ],
        ..test_api()
    }
}

fn write_core_crate_manifest(ws_root: &std::path::Path) {
    let core_dir = ws_root.join("crates").join("sample-core");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        "[package]\nname = \"sample-core\"\n\n[features]\n\
         default = []\nnative-http = []\nopendal-cache = []\nwasm-http = []\ntokenizer = []\ntower = []\n",
    )
    .expect("write core Cargo.toml");
}

fn write_existing_manifest(config: &ResolvedCrateConfig, relative: &Path) -> PathBuf {
    let manifest_path = crate::codegen::cfg::resolve_against_workspace_root(config, relative);
    std::fs::create_dir_all(manifest_path.parent().expect("manifest has a parent")).expect("create manifest dir");
    std::fs::write(&manifest_path, EXISTING_MANIFEST).expect("write existing manifest");
    manifest_path
}

/// Reproduces the sample-core incident end to end through the real orchestration entry point: both
/// the Ruby and Elixir manifests already exist, declare unrelated features, and are missing the
/// two features the generated source actually forwards. After repair, both manifests declare
/// `tokenizer` and `tower`, forwarded to the core crate the same way the pre-existing rows are,
/// and every pre-existing line survives.
#[test]
fn repair_adds_missing_features_to_both_ruby_and_elixir_manifests() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws_root = dir.path();
    let config = sample_config(ws_root);
    write_core_crate_manifest(ws_root);

    let ruby_relative = ruby_native_manifest_path(&config);
    let elixir_relative = PathBuf::from(elixir_native_crate_dir(&config)).join("Cargo.toml");
    let ruby_manifest = write_existing_manifest(&config, &ruby_relative);
    let elixir_manifest = write_existing_manifest(&config, &elixir_relative);

    let api = sample_api();
    let repaired = repair_missing_cfg_binding_features(&api, &config, &[Language::Ruby, Language::Elixir]);

    assert_eq!(
        repaired.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([ruby_manifest.clone(), elixir_manifest.clone()]),
        "both manifests must be reported as repaired"
    );

    for manifest in [&ruby_manifest, &elixir_manifest] {
        let content = std::fs::read_to_string(manifest).expect("read repaired manifest");
        assert!(
            content.contains(r#"tokenizer = ["sample-core/tokenizer"]"#),
            "{}: tokenizer must be forwarded, got:\n{content}",
            manifest.display()
        );
        assert!(
            content.contains(r#"tower = ["sample-core/tower"]"#),
            "{}: tower must be forwarded, got:\n{content}",
            manifest.display()
        );
        assert!(
            content.contains(r#"default = ["native-http", "tokenizer", "tower"]"#),
            "{}: declaring a feature is not enough -- it must also be enabled by default, or \
             #[cfg(feature = \"...\")] stays false and the compile-out this repair exists to fix \
             recurs silently, got:\n{content}",
            manifest.display()
        );
        for preserved in [
            r#"native-http = ["sample-core/native-http"]"#,
            r#"opendal-cache = ["sample-core/opendal-cache"]"#,
            r#"wasm-http = ["sample-core/wasm-http"]"#,
            "[dependencies]",
            "magnus = \"0.7\"",
        ] {
            assert!(
                content.contains(preserved),
                "{}: `{preserved}` must survive the repair, got:\n{content}",
                manifest.display()
            );
        }
    }
}

/// A language absent from the requested set must not have its manifest touched, even though it
/// is equally missing the feature -- scaffolding one language must never write another's files.
#[test]
fn repair_skips_a_manifest_for_a_language_not_requested() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws_root = dir.path();
    let config = sample_config(ws_root);
    write_core_crate_manifest(ws_root);

    let elixir_relative = PathBuf::from(elixir_native_crate_dir(&config)).join("Cargo.toml");
    let elixir_manifest = write_existing_manifest(&config, &elixir_relative);

    let api = sample_api();
    let repaired = repair_missing_cfg_binding_features(&api, &config, &[Language::Ruby]);

    assert!(
        repaired.is_empty(),
        "requesting only Ruby must not touch the Elixir manifest, but repaired: {repaired:?}"
    );
    let content = std::fs::read_to_string(&elixir_manifest).expect("read untouched manifest");
    assert_eq!(
        content, EXISTING_MANIFEST,
        "the skipped-language manifest must be byte-for-byte unchanged"
    );
}

/// A manifest that has not been scaffolded yet (no file on disk) is left for the ordinary
/// scaffold-creation path, which already derives the correct `[features]` table from
/// `collect_cfg_features`; this repair has nothing to patch and must not error or create one.
#[test]
fn repair_is_a_no_op_when_no_manifest_exists_yet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws_root = dir.path();
    let config = sample_config(ws_root);
    write_core_crate_manifest(ws_root);

    let api = sample_api();
    let repaired = repair_missing_cfg_binding_features(&api, &config, &[Language::Ruby, Language::Elixir]);

    assert!(
        repaired.is_empty(),
        "an unscaffolded crate has no manifest to repair, got: {repaired:?}"
    );
    let ruby_relative = ruby_native_manifest_path(&config);
    let ruby_manifest = crate::codegen::cfg::resolve_against_workspace_root(&config, &ruby_relative);
    assert!(
        !ruby_manifest.exists(),
        "this repair must never create a manifest that was never scaffolded"
    );
}
