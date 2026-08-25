//! End-to-end proof that `NapiBackend::generate_bindings` wires a delegating `Deserialize` for
//! a struct carrying a container-level `#[serde(from/into)]` conversion. The decision itself
//! (`struct_wants_deserialize_delegation`) is unit-tested directly against
//! `napi::gen_bindings::types::gen_struct` in `types/tests.rs`, but the *wiring* that builds
//! `core_to_binding_for_deserialize` in `napi/gen_bindings/mod.rs` and threads it through is
//! only exercised here, through the real backend entry point.
//!
//! A new file rather than an addition to `tests/backends_napi_gen_bindings_test.rs`: that file
//! is already at its `file_size_ratchet` ceiling (3771 lines) and must not grow. ~keep
use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::*;

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.node]
package_name = "test-lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn f64_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Primitive(PrimitiveType::F64),
        ..Default::default()
    }
}

#[test]
fn generate_bindings_delegates_deserialize_for_a_sound_container_conversion_struct() {
    let backend = NapiBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Point".to_string(),
            rust_path: "test_lib::Point".to_string(),
            fields: vec![f64_field("x"), f64_field("y")],
            has_serde: true,
            serde_container_conversion: SerdeContainerConversion {
                from: Some("(f64, f64)".to_string()),
                into: Some("(f64, f64)".to_string()),
                try_from: None,
                transparent: false,
            },
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut config = make_config();
    // napi's `has_serde` gate is filesystem-detected (`detect_serde_available` walks up from
    // `output_paths["node"]` looking for a Cargo.toml declaring serde) rather than IR-driven, so
    // a synthetic config with no real output directory would silently disable Serialize/
    // Deserialize entirely and make this test vacuously pass. Point it at this crate's own
    // Cargo.toml, which does declare `serde` (derive) + `serde_json`. ~keep
    config
        .output_paths
        .insert("node".to_string(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");
    let content = &lib_rs.content;

    // napi prefixes generated binding type names with the configured node type prefix, "Js" by
    // default (`ResolvedCrateConfig::node_type_prefix`) -- the emitted struct is `JsPoint`.
    let point_idx = content.find("struct JsPoint").expect("JsPoint struct must be emitted");
    let derive_line = content[..point_idx]
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("JsPoint struct must have a derive line above it");

    assert!(
        !derive_line.contains("serde::Deserialize"),
        "JsPoint's derive must drop Deserialize once generate_bindings confirms delegation eligibility: {derive_line}\nfull output:\n{content}"
    );
    assert!(
        content.contains("impl<'de> serde::Deserialize<'de> for JsPoint {"),
        "generate_bindings must emit a delegating Deserialize impl for JsPoint:\n{content}"
    );
    assert!(
        content.contains("<test_lib::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "the delegating impl must read the core type via generate_bindings' own wiring:\n{content}"
    );
}
