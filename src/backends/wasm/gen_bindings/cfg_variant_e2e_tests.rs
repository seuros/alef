//! End-to-end regression coverage for alef #536/#538: a host-owned cfg-gated enum variant run
//! through the REAL `WasmBackend::generate_bindings` path, not a direct `enums::gen_enum` call.
//!
//! Split out of `tests.rs` (which crossed the 1,000-line cap) rather than folded into it, since
//! this is a self-contained concept -- the production wiring of `configured_features` -- with its
//! own fixtures, distinct from the rest of that file's `Cargo.toml`/struct/method coverage.

use super::WasmBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

/// alef #536/#538: production once passed `None` for `configured_features` at this exact call
/// site, so `enums::gen_enum`'s declaration was always unconditional while its conversion arm
/// stayed `#[cfg(...)]`-gated -- a mismatch a unit test calling `gen_enum` directly cannot catch,
/// since it supplies `configured_features` by hand. Only `WasmBackend::generate_bindings` itself
/// proves `expand_configured_features`'s result actually reaches the enum declaration. ~keep
fn host_cfg_enum_config(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"wasm\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.wasm]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn host_cfg_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "RenderMode".to_string(),
            rust_path: "test_lib::RenderMode".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Extended".to_string(),
                    cfg: Some(r#"feature = "extended-mode""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
}

/// Slice out just the `#[wasm_bindgen] pub enum WasmRenderMode { ... }` declaration -- the ONLY
/// place a `#[cfg(...)]` attribute is actually invalid (rustwasm/wasm-bindgen#2058). A plain
/// `impl From<test_lib::RenderMode> for WasmRenderMode` conversion arm legitimately keeps its own
/// `#[cfg(...)]` guard elsewhere in the same file (it is an ordinary, non-macro-annotated `impl`
/// block matching the REAL core type, unaffected by the macro limitation), so asserting over the
/// whole file would fail on that correct, unrelated cfg and prove nothing about the declaration
/// this test exists to check. ~keep
fn wasm_render_mode_declaration(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("pub enum WasmRenderMode {")
        .expect("generated crate must declare the WasmRenderMode enum");
    let end = lib_rs[start..]
        .find('}')
        .map(|i| start + i + 1)
        .expect("enum declaration must close");
    &lib_rs[start..end]
}

#[test]
fn generate_bindings_declares_host_cfg_variant_when_feature_configured_end_to_end() {
    let api = host_cfg_enum_api();
    let config = host_cfg_enum_config(Some("extended-mode"));
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let declaration = wasm_render_mode_declaration(lib_rs);

    assert!(
        !declaration.contains("#[cfg("),
        "no wasm_bindgen enum variant may ever carry a #[cfg(...)] attribute \
         (rustwasm/wasm-bindgen#2058), got:\n{declaration}"
    );
    assert!(
        declaration.contains("Extended"),
        "the configured variant must reach the generated crate through the real \
         WasmBackend::generate_bindings path, got:\n{declaration}"
    );
}

#[test]
fn generate_bindings_omits_host_cfg_variant_when_feature_not_configured_end_to_end() {
    let api = host_cfg_enum_api();
    let config = host_cfg_enum_config(None);
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let declaration = wasm_render_mode_declaration(lib_rs);

    assert!(
        !declaration.contains("#[cfg("),
        "no wasm_bindgen enum variant may ever carry a #[cfg(...)] attribute \
         (rustwasm/wasm-bindgen#2058), got:\n{declaration}"
    );
    assert!(
        !declaration.contains("Extended"),
        "an unconfigured host-owned variant must not reach the generated crate at all through the \
         real WasmBackend::generate_bindings path, got:\n{declaration}"
    );
}
