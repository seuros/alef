//! End-to-end proof that `MagnusBackend::generate_bindings` actually wires a delegating
//! `Deserialize` for a struct carrying a container-level `#[serde(from/into)]` conversion.
//!
//! Magnus is the one delegating-deserialize backend whose eligibility decision
//! (`can_generate_conversion(...) && struct_wants_deserialize_delegation(...)`) lives in
//! `backends/magnus/gen_bindings/mod.rs`, not inside `classes::gen_struct` itself --
//! `classes::gen_struct` takes the already-computed `delegate_deserialize: bool` as a plain
//! parameter. The existing unit tests in `classes/tests.rs` only prove the *rendering* is
//! correct given that bool; they never call through `generate_bindings`, so a bug in the
//! `mod.rs` wiring (e.g. reusing the wrong convertible-type set, or dropping the
//! `struct_wants_deserialize_delegation` call) would pass every existing magnus test while
//! still shipping the original silent-decode bug. This test exercises the real backend
//! entry point end to end to close that gap. ~keep
use alef::backends::magnus::MagnusBackend;
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
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
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
    let backend = MagnusBackend;

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
    let lib_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .expect("generate_bindings must include lib.rs");
    let content = &lib_file.content;

    // Only one type is in this ApiSurface, so the derive line immediately preceding
    // `struct Point` is unambiguous.
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
        derive_line.contains("serde::Serialize"),
        "Serialize stays derived (out of scope for this fix): {derive_line}"
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
