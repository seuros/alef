//! Coverage for `assert_vtable_matches_rust_struct`: the Java trait-bridge vtable slot check.
//!
//! Split out of `gen_bindings/mod.rs`, which sits over the repo's 1,000-line cap (see
//! `file-modularization` in CLAUDE.md). ~keep

use super::*;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};

fn ocr_shaped_trait() -> TypeDef {
    TypeDef {
        name: "OcrBackend".into(),
        rust_path: "sample_core::OcrBackend".into(),
        is_trait: true,
        methods: vec![
            MethodDef {
                name: "supports_language".into(),
                params: vec![crate::core::ir::ParamDef {
                    name: "lang".into(),
                    ty: TypeRef::String,
                    ..crate::core::ir::ParamDef::default()
                }],
                return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                ..MethodDef::default()
            },
            MethodDef {
                name: "backend_type".into(),
                return_type: TypeRef::Named("OcrBackendType".into()),
                ..MethodDef::default()
            },
            MethodDef {
                name: "supported_languages".into(),
                return_type: TypeRef::Vec(Box::new(TypeRef::String)),
                ..MethodDef::default()
            },
        ],
        ..TypeDef::default()
    }
}

fn ocr_bridge_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "OcrBackend".into(),
            super_trait: Some("Plugin".into()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

/// Regression: `[crates.java].exclude_types` (and every other source feeding
/// `effective_exclude_types`) must not delete a trait method from the bridge. The
/// method keeps a slot in the Rust vtable struct, so deleting it here leaves Java
/// writing N-1 upcall stubs into an N-slot struct.
#[test]
fn excluded_return_type_does_not_remove_a_vtable_slot() {
    let api = ApiSurface {
        crate_name: "sample".into(),
        types: vec![
            ocr_shaped_trait(),
            TypeDef {
                name: "OcrBackendType".into(),
                binding_excluded: true,
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let files = JavaBackend
        .generate_bindings(&api, &ocr_bridge_config())
        .expect("Java bindings");
    let bridge = files
        .iter()
        .find(|file| file.path.ends_with("OcrBackendBridge.java"))
        .expect("bridge class");

    let stub_writes: Vec<&str> = bridge
        .content
        .lines()
        .filter(|line| line.contains("ValueLayout.ADDRESS.byteSize());"))
        .map(str::trim)
        .collect();
    assert_eq!(
        stub_writes,
        vec![
            "initStubName(0L * ValueLayout.ADDRESS.byteSize());",
            "initStubVersion(1L * ValueLayout.ADDRESS.byteSize());",
            "initStubInitialize(2L * ValueLayout.ADDRESS.byteSize());",
            "initStubShutdown(3L * ValueLayout.ADDRESS.byteSize());",
            "initStubSupportsLanguage(4L * ValueLayout.ADDRESS.byteSize());",
            "initStubBackendType(5L * ValueLayout.ADDRESS.byteSize());",
            "initStubSupportedLanguages(6L * ValueLayout.ADDRESS.byteSize());",
            "initStubFreeString(7L * ValueLayout.ADDRESS.byteSize());",
            "initStubFreeUserData(8L * ValueLayout.ADDRESS.byteSize());",
        ],
        "every Rust vtable field must get a stub, at its own index"
    );
}

#[test]
fn vtable_slot_check_accepts_a_faithful_bridge() {
    let trait_def = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![trait_def.clone()],
        ..ApiSurface::default()
    };
    let emitted = crate::codegen::generators::trait_bridge::vtable_slot_names(&trait_def, true, &[]);

    assert_vtable_matches_rust_struct(&api, &trait_def, true, &[], &emitted).expect("matching slot lists must pass");
}

#[test]
fn vtable_slot_check_rejects_a_dropped_slot() {
    let source_trait = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![source_trait.clone()],
        ..ApiSurface::default()
    };
    let mut pruned_trait = source_trait.clone();
    pruned_trait.methods.retain(|method| method.name != "backend_type");
    let emitted = crate::codegen::generators::trait_bridge::vtable_slot_names(&pruned_trait, true, &[]);

    let error = assert_vtable_matches_rust_struct(&api, &pruned_trait, true, &[], &emitted)
        .expect_err("a bridge missing a slot must fail generation");
    let message = error.to_string();
    assert!(
        message.contains("Rust slots (9)") && message.contains("Java slots (8)"),
        "the failure must report both slot counts;\nactual:\n{message}"
    );
    assert!(
        message.contains("backend_type"),
        "the failure must name the slots that disagree;\nactual:\n{message}"
    );
}

#[test]
fn vtable_slot_check_rejects_a_reordered_slot() {
    let trait_def = ocr_shaped_trait();
    let api = ApiSurface {
        types: vec![trait_def.clone()],
        ..ApiSurface::default()
    };
    let mut reordered = crate::codegen::generators::trait_bridge::vtable_slot_names(&trait_def, true, &[]);
    reordered.swap(5, 6);

    let error = assert_vtable_matches_rust_struct(&api, &trait_def, true, &[], &reordered)
        .expect_err("a bridge with the right slot count in the wrong order must fail generation");
    assert!(
        error.to_string().contains("backend_type"),
        "the failure must show the emitted order;\nactual:\n{error}"
    );
}
