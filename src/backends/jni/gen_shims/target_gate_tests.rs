#[cfg(test)]
mod target_gate_tests {
    use super::*;

    /// Write a core crate manifest whose `[features]` table defines an aggregate over a leaf,
    /// and return a resolved config rooted at it.
    ///
    /// `alef.toml` names the aggregate -- never the leaf -- exactly as a consumer would when the
    /// Android build is `cargo ndk ... --no-default-features --features <aggregate>`.
    fn config_with_core_aggregate(directory: &std::path::Path, alef_toml: &str) -> ResolvedCrateConfig {
        let core_dir = directory.join("crates").join("demo");
        std::fs::create_dir_all(&core_dir).expect("create core crate dir");
        std::fs::write(
            core_dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n\n[features]\ndefault = []\nmobile-target = [\"decoder\"]\ndecoder = []\n",
        )
        .expect("write core Cargo.toml");

        let raw: crate::core::config::NewAlefConfig = toml::from_str(alef_toml).expect("fixture config parses");
        let mut config = raw.resolve().expect("fixture config resolves").remove(0);
        config.workspace_root = Some(directory.to_path_buf());
        config
    }

    /// Like [`config_with_core_aggregate`], but the core crate's `[features] default` list is
    /// caller-supplied so a test can turn a gate on through the crate's own defaults rather than
    /// through any configured name.
    fn config_with_core_defaults(directory: &std::path::Path, defaults: &str, alef_toml: &str) -> ResolvedCrateConfig {
        let core_dir = directory.join("crates").join("demo");
        std::fs::create_dir_all(&core_dir).expect("create core crate dir");
        std::fs::write(
            core_dir.join("Cargo.toml"),
            format!("[package]\nname = \"demo\"\n\n[features]\ndefault = [{defaults}]\ndecoder = []\n"),
        )
        .expect("write core Cargo.toml");

        let raw: crate::core::config::NewAlefConfig = toml::from_str(alef_toml).expect("fixture config parses");
        let mut config = raw.resolve().expect("fixture config resolves").remove(0);
        config.workspace_root = Some(directory.to_path_buf());
        config
    }

    fn gated_function() -> crate::core::ir::FunctionDef {
        crate::core::ir::FunctionDef {
            name: "decoder_details".into(),
            rust_path: "demo::decoder::decoder_details".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"decoder\"".into()),
            ..Default::default()
        }
    }

    fn api_with(functions: Vec<crate::core::ir::FunctionDef>) -> ApiSurface {
        ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            functions,
            ..Default::default()
        }
    }

    /// A `[[crates.jni.target_dep_overrides]]` entry that names a core-crate AGGREGATE feature
    /// must satisfy the gates of every member that aggregate enables.
    ///
    /// `cfg_feature_satisfied` matches gate names against the configured list literally and
    /// hard-codes exactly one umbrella name (`full`), so an override configured as
    /// `features = ["mobile-target"]` used to leave `#[cfg(feature = "decoder")]` unsatisfied on
    /// that target -- even though cargo resolves `mobile-target = ["decoder"]` and the
    /// cross-compiled library really does export the symbol. The shim was then emitted behind
    /// `#[cfg(not(any(<target>)))]`, so the Android artifact silently lost the function while
    /// every desktop target kept it: underexposure with no diagnostic anywhere. ~keep
    #[test]
    fn target_override_naming_a_core_aggregate_keeps_its_member_gated_shims() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = config_with_core_aggregate(
            directory.path(),
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["mobile-target"]
"#,
        );

        let content = emit_lib_rs(&api_with(vec![gated_function()]), &config);

        assert!(
            content.contains("core_crate::decoder::decoder_details()"),
            "the aggregate enables `decoder`, so its shim must be emitted at all: {content}"
        );
        assert!(
            !content.contains("#[cfg(not(any(target_os = \"android\")))]"),
            "the aggregate enables `decoder` on the override target too, so the shim must NOT be \
             excluded from it: {content}"
        );
    }

    /// The expansion must not turn the gate check into a rubber stamp: a leaf the configured
    /// aggregate does NOT enable stays unsatisfied on that target, and its shim stays gated off
    /// it.
    #[test]
    fn target_override_still_excludes_a_leaf_its_aggregate_does_not_enable() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = config_with_core_aggregate(
            directory.path(),
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["mobile-target"]
"#,
        );
        let desktop_only = crate::core::ir::FunctionDef {
            name: "render_preview".into(),
            rust_path: "demo::preview::render_preview".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"preview\"".into()),
            ..Default::default()
        };

        let content = emit_lib_rs(&api_with(vec![desktop_only]), &config);

        assert!(
            content.contains("#[cfg(not(any(target_os = \"android\")))]"),
            "`preview` is not a member of `mobile-target`, so its shim must stay gated off the \
             override target: {content}"
        );
    }

    /// The default (non-overridden) branch's core dep is emitted by `scaffold::languages::jni`
    /// through `render_core_dep` with no `default-features = false`, so the core crate's own
    /// `default = [...]` list is always active there. Deriving the branch's enabled set from the
    /// configured `features` list alone understates it, and a crate that turns a feature on by
    /// default lost every shim behind that gate on every target.
    #[test]
    fn the_default_branch_counts_the_core_crates_own_default_features() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = config_with_core_defaults(
            directory.path(),
            "\"decoder\"",
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = []
"#,
        );

        let content = emit_lib_rs(&api_with(vec![gated_function()]), &config);

        assert!(
            content.contains("core_crate::decoder::decoder_details()"),
            "the core crate enables `decoder` by default and this branch never passes \
             `default-features = false`, so the shim must be emitted: {content}"
        );
    }

    /// `FfiTargetDepOverride::default_features = true` makes the scaffold omit that branch's
    /// `default-features = false`, so the core crate's declared defaults are active on that
    /// target in addition to the override's own `features` list. Reading only the literal
    /// `features` list left the branch looking empty, so the shim was emitted behind
    /// `#[cfg(not(any(<target>)))]` — present everywhere except the one target the override
    /// exists to describe.
    #[test]
    fn an_override_that_keeps_default_features_counts_the_core_crates_defaults() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = config_with_core_defaults(
            directory.path(),
            "\"decoder\"",
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = []

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = []
default_features = true
"#,
        );

        let content = emit_lib_rs(&api_with(vec![gated_function()]), &config);

        assert!(
            content.contains("core_crate::decoder::decoder_details()"),
            "the shim must be emitted at all: {content}"
        );
        assert!(
            !content.contains("#[cfg(not(any(target_os = \"android\")))]"),
            "the override keeps the core crate's defaults, which include `decoder`, so the shim \
             must NOT be excluded from the override target: {content}"
        );
    }

    /// The union must stay conditional on the flag: an override that opts out of the core dep's
    /// default features does not get them, and a gate only those defaults satisfy stays off that
    /// target.
    #[test]
    fn an_override_without_default_features_does_not_get_the_core_crates_defaults() {
        let directory = tempfile::tempdir().expect("tempdir");
        let config = config_with_core_defaults(
            directory.path(),
            "\"decoder\"",
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["crates/demo/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
features = []

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = []
default_features = false
"#,
        );

        let content = emit_lib_rs(&api_with(vec![gated_function()]), &config);

        assert!(
            content.contains("#[cfg(not(any(target_os = \"android\")))]"),
            "this override passes `default-features = false` and names no features, so nothing \
             satisfies `decoder` on that target and the shim must stay gated off it: {content}"
        );
    }
}
