//! End-to-end proof that `RustlerBackend::generate_bindings` wires a delegating `Deserialize`
//! for a struct carrying a container-level `#[serde(from/into)]` conversion. The decision
//! itself (`struct_wants_deserialize_delegation`) is unit-tested directly against
//! `rustler::gen_bindings::types::gen_struct` in `types/tests.rs`, but the *wiring* that builds
//! `core_to_binding_for_deserialize` in `rustler/gen_bindings/native.rs` and threads it through
//! is only exercised here, through the real backend entry point.
//!
//! A new file rather than an addition to `tests/backends_rustler_gen_public_api_test.rs`: that
//! file is already at its `file_size_ratchet` ceiling (2244 lines) and must not grow. ~keep
use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::*;

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "test_lib"
"#,
    )
    .expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
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
    let backend = RustlerBackend;

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
        // rustler's `collect_types_for_nif_derives` only emits a struct that is transitively
        // reachable from a function/method param or return type -- an ApiSurface with no
        // function referencing `Point` would silently make this test vacuously pass (Point
        // never gets emitted at all, so the "no Deserialize in the derive line" assertion below
        // could never fail either). This function makes `Point` reachable. ~keep
        functions: vec![FunctionDef {
            name: "make_point".to_string(),
            rust_path: "test_lib::make_point".to_string(),
            return_type: TypeRef::Named("Point".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let config = make_config();
    let files = backend
        .generate_bindings(&api, &config)
        .expect("generate_bindings failed");
    let native_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("native.rs") || f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must include a Rust source file with Point");
    let content = &native_rs.content;

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
