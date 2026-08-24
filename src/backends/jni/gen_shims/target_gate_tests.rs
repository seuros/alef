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
}
