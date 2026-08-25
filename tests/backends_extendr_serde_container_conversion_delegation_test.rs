//! End-to-end proof that `ExtendrBackend::generate_bindings` wires a delegating `Deserialize`
//! for a struct carrying a container-level `#[serde(from/into)]` conversion. extendr's own
//! delegation decision lives in `codegen::generators::structs` (already covered at the unit
//! level in `structs/tests.rs`), but the *wiring* -- `cfg.delegate_deserialize_to_core_for_types
//! = Some(&core_to_binding_for_deserialize)` in `extendr/gen_bindings/mod.rs` -- is only
//! exercised here, through the real backend entry point.
//!
//! A new file rather than an addition to `tests/backends_extendr_gen_bindings_test.rs`: that
//! file is already at its `file_size_ratchet` ceiling (2142 lines) and must not grow. ~keep
use alef::backends::extendr::ExtendrBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"
"#,
    )
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
    let backend = ExtendrBackend;

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

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include lib.rs");
    let content = &lib_rs.content;

    let point_idx = content.find("struct Point").expect("Point struct must be emitted");
    let derive_line = content[..point_idx]
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("Point struct must have a derive line above it");

    assert!(
        !derive_line.contains("serde::Deserialize"),
        "Point's derive must drop Deserialize once generate_bindings confirms delegation eligibility: {derive_line}\nfull output:\n{content}"
    );
    assert!(
        content.contains("impl<'de> serde::Deserialize<'de> for Point {"),
        "generate_bindings must emit a delegating Deserialize impl for Point:\n{content}"
    );
    assert!(
        content.contains("<test_lib::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "the delegating impl must read the core type via generate_bindings' own wiring:\n{content}"
    );
}
