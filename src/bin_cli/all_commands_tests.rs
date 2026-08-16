use super::{
    snippet_validation_needs_build_artifacts, sync_registry_versions_before_all, warn_if_snippet_validation_needs_build,
};
use crate::core::config::NewAlefConfig;

#[test]
fn all_generates_snippets_before_readmes_consume_them() {
    let source = include_str!("all_commands.rs");
    let e2e = source.find("Generating e2e test suites...").expect("e2e stage");
    let readmes = source.find("Generating READMEs...").expect("README stage");

    assert!(
        e2e < readmes,
        "README generation must observe snippets produced by the same run"
    );
}

#[test]
fn all_runs_its_only_build_step_before_the_docs_stage_that_validates_snippets() {
    let source = include_str!("all_commands.rs");
    let post_build = source
        .find("Running post-build processing...")
        .expect("post-build stage");
    let docs = source.find("Generating docs...").expect("docs stage");

    assert!(
        post_build < docs,
        "the only build `all` performs (FFI cdylib + per-backend post-build hooks, via \
         complete_generated_artifacts) runs before the docs stage that triggers snippet validation -- \
         but that build is FFI-only and does not satisfy typecheck/compile/run snippet validation for \
         languages needing a full per-language build (typescript, java, kotlin, swift, zig, ...)"
    );
}

#[test]
fn all_never_calls_the_general_per_language_build_stage() {
    let source = include_str!("all_commands.rs");

    assert!(
        !source.contains("pipeline::build("),
        "`alef all`'s documented scope (\"generate + stubs + scaffold + readme + docs + sync + e2e\") \
         excludes building native artifacts; the only build all_commands.rs may trigger is the narrow \
         FFI-only one inside `complete_generated_artifacts`. If this now calls `pipeline::build` \
         directly, `warn_if_snippet_validation_needs_build`'s precondition warning (and its doc \
         comment) is stale and must be revisited alongside this test."
    );
}

#[test]
fn snippet_validation_needs_build_artifacts_is_true_only_for_toolchain_levels() {
    assert!(snippet_validation_needs_build_artifacts(Some("typecheck")));
    assert!(snippet_validation_needs_build_artifacts(Some("compile")));
    assert!(snippet_validation_needs_build_artifacts(Some("run")));
    assert!(
        snippet_validation_needs_build_artifacts(Some("Compile")),
        "the check must be case-insensitive since config values are user-authored TOML strings"
    );

    assert!(!snippet_validation_needs_build_artifacts(Some("syntax")));
    assert!(!snippet_validation_needs_build_artifacts(None));
    assert!(!snippet_validation_needs_build_artifacts(Some("bogus")));
}

fn write_config_with_snippet_validation_level(root: &std::path::Path, validation_level: &str) -> std::path::PathBuf {
    let cargo_path = root.join("Cargo.toml");
    std::fs::write(&cargo_path, "[package]\nname = \"sample-core\"\nversion = \"0.1.0\"\n").expect("write Cargo.toml");
    let config_path = root.join("alef.toml");
    let config = format!(
        concat!(
            "[workspace]\nlanguages = [\"zig\"]\n\n",
            "[workspace.docs.snippets]\nvalidation_level = {:?}\n\n",
            "[[crates]]\nname = \"sample-core\"\nsources = []\nversion_from = {:?}\n"
        ),
        validation_level,
        cargo_path.to_string_lossy(),
    );
    std::fs::write(&config_path, config).expect("write alef.toml");
    config_path
}

#[test]
fn warn_if_snippet_validation_needs_build_reads_the_merged_validation_level_without_panicking() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_path = write_config_with_snippet_validation_level(temp.path(), "compile");
    let configs = resolve(&config_path);
    let config = configs.into_iter().next().expect("one crate");

    let merged_level = config
        .docs
        .as_ref()
        .and_then(|docs| docs.snippets.as_ref())
        .and_then(|snippets| snippets.validation_level.as_deref());
    assert_eq!(merged_level, Some("compile"));

    // Only exercises the tracing::warn! side effect for panics; no subscriber is
    // installed in this test so nothing is asserted about the emitted message itself.
    warn_if_snippet_validation_needs_build(&config);
}

fn write_neutral_config(root: &std::path::Path, cargo_toml: &str, hash: &str) -> std::path::PathBuf {
    let cargo_path = root.join("Cargo.toml");
    std::fs::write(&cargo_path, cargo_toml).expect("write Cargo.toml");
    let config_path = root.join("alef.toml");
    let config = format!(
        concat!(
            "[workspace]\nlanguages = [\"zig\"]\n\n",
            "[[crates]]\nname = \"sample-core\"\nsources = []\nversion_from = {:?}\n\n",
            "[crates.e2e.call]\nfunction = \"sample_call\"\n\n",
            "[crates.e2e.registry.packages.zig]\n",
            "name = \"sample_pkg\"\nversion = \"0.8.0\"\nhash = {:?}\n"
        ),
        cargo_path.to_string_lossy(),
        hash
    );
    std::fs::write(&config_path, config).expect("write alef.toml");
    config_path
}

fn resolve(config_path: &std::path::Path) -> Vec<crate::core::config::ResolvedCrateConfig> {
    let raw = std::fs::read_to_string(config_path).expect("read alef.toml");
    toml::from_str::<NewAlefConfig>(&raw)
        .expect("parse alef.toml")
        .resolve()
        .expect("resolve alef.toml")
}

#[test]
fn all_preflight_repairs_stale_zig_registry_hash_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stale_hash = "sample_pkg-0.8.0-AbCd_XyZ123456789";
    let config_path = write_neutral_config(
        temp.path(),
        "[package]\nname = \"sample-core\"\nversion = \"0.9.0\"\n",
        stale_hash,
    );
    let configs = resolve(&config_path);
    let selected = configs.iter().collect::<Vec<_>>();

    let changed = sync_registry_versions_before_all(&config_path, &selected).expect("repair stale hash");

    assert!(changed);
    let repaired = std::fs::read_to_string(config_path).expect("read repaired config");
    assert!(repaired.contains("version = \"0.9.0\""));
    assert!(repaired.contains("hash = \"sample_pkg-0.9.0-AbCd_XyZ123456789\""));
}

#[test]
fn all_preflight_rejects_unreadable_version_source_without_mutating_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stale_hash = "sample_pkg-0.8.0-AbCd_XyZ123456789";
    let config_path = write_neutral_config(temp.path(), "not valid TOML", stale_hash);
    let configs = resolve(&config_path);
    let selected = configs.iter().collect::<Vec<_>>();

    let error = sync_registry_versions_before_all(&config_path, &selected).expect_err("invalid version must fail");

    assert!(
        error
            .to_string()
            .contains("could not resolve version for crate `sample-core`")
    );
    let unchanged = std::fs::read_to_string(config_path).expect("read unchanged config");
    assert!(unchanged.contains(stale_hash));
}
