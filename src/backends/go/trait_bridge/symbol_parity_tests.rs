//! Parity between the trait-bridge C symbols Go *calls* and the ones the FFI backend *exports*.
//!
//! The register symbol is not derived from the trait name on the FFI side: it is
//! `{prefix}_{register_fn}`, straight from `[[crates.trait_bridges]]`. Go composed
//! `{prefix}_register_{trait_snake}`, which agrees only when the configured name happens to spell
//! that. These tests assert against `ffi::trait_bridge::registration_surface` — the FFI backend's
//! own report of what it exported — rather than against a string Go happens to produce today.

use super::orchestration::gen_trait_bridge;
use crate::core::backend::TraitBridgeRegistrationSurface;
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::HashSet;

const PREFIX: &str = "demo";

fn trait_def(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo_core::{name}"),
        is_trait: true,
        ..Default::default()
    }
}

fn bridge_config(trait_name: &str, register_fn: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        registry_getter: Some("demo_core::registry".to_string()),
        register_fn: register_fn.map(str::to_string),
        unregister_fn: None,
        clear_fn: None,
        ..Default::default()
    }
}

fn generate_go(trait_name: &str, register_fn: Option<&str>) -> String {
    let mut out = String::new();
    let excluded: HashSet<&str> = HashSet::new();
    gen_trait_bridge(
        &mut out,
        &trait_def(trait_name),
        &bridge_config(trait_name, register_fn),
        PREFIX,
        &excluded,
        "",
    );
    out
}

/// The symbols the FFI backend reports having exported for this bridge.
fn exported_surface(trait_name: &str, register_fn: Option<&str>) -> TraitBridgeRegistrationSurface {
    let api = ApiSurface {
        types: vec![trait_def(trait_name)],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: PREFIX.to_string(),
        trait_bridges: vec![bridge_config(trait_name, register_fn)],
        ..ResolvedCrateConfig::default()
    };
    crate::backends::ffi::trait_bridge::registration_surface(&api, &config)
        .into_iter()
        .next()
        .expect("the FFI backend exports a registration surface for a bridge with a register_fn")
}

/// Every `C.<symbol>(` call site in the generated Go source.
fn called_c_symbols(generated: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut rest = generated;
    while let Some(at) = rest.find("C.") {
        rest = &rest[at + 2..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let (symbol, tail) = rest.split_at(end);
        if tail.starts_with('(') && !symbol.is_empty() {
            symbols.push(symbol.to_string());
        }
        rest = tail;
    }
    symbols
}

/// `register_fn` names whose spelling is *not* `register_{trait_snake}`. A configured name that
/// happens to match the trait-derived form would pass under either derivation and prove
/// nothing. ~keep
const DIVERGENT_REGISTER_FNS: &[(&str, &str)] = &[
    ("OcrBackend", "install_backend"),
    ("HTTPClient", "add_http_client"),
    ("UTF8Decoder", "register_decoder"),
    ("_Internal", "attach_internal"),
];

#[test]
fn should_call_the_exported_register_symbol_when_register_fn_is_not_trait_derived() {
    for (trait_name, register_fn) in DIVERGENT_REGISTER_FNS {
        let generated = generate_go(trait_name, Some(register_fn));
        let exported = exported_surface(trait_name, Some(register_fn))
            .register_symbol
            .expect("register_fn is configured");

        assert!(
            called_c_symbols(&generated).contains(&exported),
            "`{trait_name}` calls {:?}, but the FFI backend exports `{exported}`",
            called_c_symbols(&generated)
        );
    }
}

/// Control for the test above: the rows must actually separate the two derivations. Go used to
/// compose `{prefix}_register_{trait_snake}`; on a `register_fn` that already spells that, both
/// derivations produce the same string and the parity test would pass without proving
/// anything. ~keep
#[test]
fn should_not_spell_the_register_symbol_the_way_the_trait_derivation_would() {
    let generated = generate_go("OcrBackend", Some("install_backend"));
    let trait_derivation = format!("{PREFIX}_register_ocr_backend");

    assert_eq!(
        exported_surface("OcrBackend", Some("install_backend")).register_symbol,
        Some("demo_install_backend".to_string())
    );
    assert!(
        !called_c_symbols(&generated).contains(&trait_derivation),
        "still emitting `{trait_derivation}`, which is exported nowhere:\n{generated}"
    );
}

#[test]
fn should_call_the_exported_unregister_symbol_when_the_trait_name_defeats_snake_casing() {
    for (trait_name, register_fn) in DIVERGENT_REGISTER_FNS {
        let generated = generate_go(trait_name, Some(register_fn));
        let exported = exported_surface(trait_name, Some(register_fn))
            .unregister_symbol
            .expect("unregister is always emitted alongside register");

        assert!(
            called_c_symbols(&generated).contains(&exported),
            "`{trait_name}` calls {:?}, but the FFI backend exports `{exported}`",
            called_c_symbols(&generated)
        );
    }
}

/// The vtable constructor is a cgo `static inline` helper Go declares in its own preamble, not an
/// FFI export. Its two sites — the preamble declaration and the call — must agree with each
/// other; asking the shared helper is what makes that structural rather than coincidental. ~keep
#[test]
fn go_vtable_constructor_symbol_is_not_an_exported_ffi_symbol() {
    let generated = generate_go("OcrBackend", Some("install_backend"));
    let constructor = crate::backends::go::c_symbols::go_vtable_constructor_symbol(PREFIX, "OcrBackend");

    assert_eq!(constructor, "demo_ocr_backend_vtable_new");
    assert!(
        called_c_symbols(&generated).contains(&constructor),
        "the vtable allocation must call the helper Go declares, got {:?}",
        called_c_symbols(&generated)
    );

    let surface = exported_surface("OcrBackend", Some("install_backend"));
    assert_ne!(surface.register_symbol, Some(constructor.clone()));
    assert_ne!(surface.unregister_symbol, Some(constructor));
}
