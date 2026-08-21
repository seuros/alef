//! Regression: every `EntryPoint` a C# `[DllImport]` names must actually be exported by the
//! generated C FFI crate.
//!
//! The two backends derive their symbol sets independently, so they can drift apart silently — a
//! P/Invoke naming a symbol the native library never exports compiles, passes every parse-based
//! gate, and only fails at the first call with `EntryPointNotFoundException`.
//!
//! The concrete drift this pins: a fieldless `Copy` enum crosses the C ABI as `int32_t`, never as
//! an `AlefHandle`, so the FFI backend gives it `from_i32`/`from_str` and deliberately emits no
//! `{prefix}_{enum}_from_json`. The C# `NativeMethods` emitter used to gate `FromJson` only on
//! "is this an opaque struct", so any enum reaching a parameter position got a `{E}FromJson`
//! declaration bound to a symbol that does not exist.
//!
//! The subset assertion is deliberately generic rather than a hard-coded symbol name: it is the
//! check that would have caught this class of bug without knowing which symbol regressed.

use std::collections::BTreeSet;

use alef::backends::csharp::CsharpBackend;
use alef::backends::ffi::FfiBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

const PREFIX: &str = "smp";

fn config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["ffi", "csharp"]

[[crates]]
name = "samplelib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "smp"

[crates.csharp]
namespace = "SampleLib"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn unit_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        ..Default::default()
    }
}

/// `NodeKind` is fieldless + `Copy`, so it crosses the C ABI as a scalar `int32_t`.
/// It appears both as a struct field and as a method parameter — the two positions that
/// respectively drive the FFI `free`/`to_json` emission and the C# `FromJson` emission.
fn scalar_enum_api() -> ApiSurface {
    let node_kind = EnumDef {
        name: "NodeKind".to_string(),
        rust_path: "samplelib::NodeKind".to_string(),
        variants: vec![unit_variant("Text"), unit_variant("Element")],
        doc: "Kind of a document node.".to_string(),
        is_copy: true,
        has_serde: true,
        ..Default::default()
    };

    let describe = MethodDef {
        name: "describe".to_string(),
        params: vec![ParamDef {
            name: "kind".to_string(),
            ty: TypeRef::Named("NodeKind".to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::String,
        is_static: false,
        receiver: Some(ReceiverKind::Ref),
        doc: "Describe a node kind.".to_string(),
        ..Default::default()
    };

    let cursor = TypeDef {
        name: "Cursor".to_string(),
        rust_path: "samplelib::Cursor".to_string(),
        fields: vec![],
        methods: vec![describe],
        is_opaque: true,
        doc: "Document cursor.".to_string(),
        ..Default::default()
    };

    ApiSurface {
        crate_name: "samplelib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![cursor],
        enums: vec![node_kind],
        ..Default::default()
    }
}

/// `Verdict` is data-carrying (the `Rejected` variant has a field), so it crosses the C ABI as
/// an `AlefHandle`, never a scalar `int32_t`. It reaches a parameter position ONLY through a
/// method (`Reviewer::explain`) — no free function ever consumes it — which is the exact blind
/// spot task #54 fixed in the FFI backend's `enum_pointer_param` scan: that set used to be built
/// solely from `api.functions[].params`, so an enum reached only through
/// `TypeDef::methods[].params` got no `{prefix}_{enum}_from_json` even though the C# emitter
/// above (see `opaque_param_types` in `src/backends/csharp/gen_bindings/functions.rs`) already
/// walked both free functions and methods and declared the P/Invoke unconditionally. ~keep
fn data_carrying_enum_via_method_api() -> ApiSurface {
    let verdict = EnumDef {
        name: "Verdict".to_string(),
        rust_path: "samplelib::Verdict".to_string(),
        variants: vec![
            unit_variant("Approved"),
            EnumVariant {
                name: "Rejected".to_string(),
                fields: vec![FieldDef {
                    name: "reason".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        doc: "Outcome of a review.".to_string(),
        is_copy: false,
        has_serde: true,
        ..Default::default()
    };

    let explain = MethodDef {
        name: "explain".to_string(),
        params: vec![ParamDef {
            name: "verdict".to_string(),
            ty: TypeRef::Named("Verdict".to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::String,
        is_static: false,
        receiver: Some(ReceiverKind::Ref),
        doc: "Explain a verdict.".to_string(),
        ..Default::default()
    };

    let reviewer = TypeDef {
        name: "Reviewer".to_string(),
        rust_path: "samplelib::Reviewer".to_string(),
        fields: vec![],
        methods: vec![explain],
        is_opaque: true,
        doc: "Reviews text.".to_string(),
        ..Default::default()
    };

    ApiSurface {
        crate_name: "samplelib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![reviewer],
        enums: vec![verdict],
        ..Default::default()
    }
}

/// Every `extern "C" fn` the generated FFI crate actually gives C linkage.
fn ffi_exported_symbols(api: &ApiSurface) -> BTreeSet<String> {
    let files = FfiBackend.generate_bindings(api, &config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let mut symbols = BTreeSet::new();
    for fragment in lib.content.split("extern \"C\" fn ").skip(1) {
        let name: String = fragment
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.starts_with(PREFIX) {
            symbols.insert(name);
        }
    }
    symbols
}

/// Every symbol a C# `[DllImport(..., EntryPoint = "...")]` binds.
fn csharp_entry_points(api: &ApiSurface) -> BTreeSet<String> {
    let files = CsharpBackend.generate_bindings(api, &config()).unwrap();

    let mut symbols = BTreeSet::new();
    for file in &files {
        for fragment in file.content.split("EntryPoint = \"").skip(1) {
            if let Some(name) = fragment.split('"').next()
                && name.starts_with(PREFIX)
            {
                symbols.insert(name.to_string());
            }
        }
    }
    symbols
}

#[test]
fn ffi_omits_from_json_for_a_scalar_crossing_enum() {
    let exported = ffi_exported_symbols(&scalar_enum_api());

    assert!(
        !exported.contains("smp_node_kind_from_json"),
        "a fieldless Copy enum crosses as int32_t, so the FFI must not export a handle-returning \
         `from_json` for it; exported: {exported:?}"
    );
    assert!(
        exported.contains("smp_node_kind_from_i32"),
        "the FFI must still export the scalar constructor it does use; exported: {exported:?}"
    );
}

#[test]
fn csharp_does_not_declare_from_json_for_a_scalar_crossing_enum() {
    let entry_points = csharp_entry_points(&scalar_enum_api());

    assert!(
        !entry_points.contains("smp_node_kind_from_json"),
        "C# must not declare a P/Invoke for `smp_node_kind_from_json` — the FFI never exports it, \
         so calling it throws EntryPointNotFoundException; declared: {entry_points:?}"
    );
}

#[test]
fn every_csharp_entry_point_is_exported_by_the_ffi_crate() {
    let api = scalar_enum_api();
    let exported = ffi_exported_symbols(&api);
    let entry_points = csharp_entry_points(&api);

    assert!(
        !entry_points.is_empty(),
        "fixture produced no C# P/Invokes, so this test would pass vacuously"
    );

    let undeclared: Vec<&String> = entry_points.difference(&exported).collect();
    assert!(
        undeclared.is_empty(),
        "C# binds symbols the FFI crate does not export: {undeclared:?}\nexported: {exported:?}"
    );
}

#[test]
fn ffi_and_csharp_agree_on_from_json_for_a_data_carrying_enum_reached_only_via_method_param() {
    let api = data_carrying_enum_via_method_api();
    let exported = ffi_exported_symbols(&api);
    let entry_points = csharp_entry_points(&api);

    assert!(
        exported.contains("smp_verdict_from_json"),
        "a data-carrying enum reached only through a method parameter must still get an FFI \
         `_from_json` export; exported: {exported:?}"
    );
    assert!(
        entry_points.contains("smp_verdict_from_json"),
        "C# must declare the matching P/Invoke; declared: {entry_points:?}"
    );

    let undeclared: Vec<&String> = entry_points.difference(&exported).collect();
    assert!(
        undeclared.is_empty(),
        "C# binds symbols the FFI crate does not export: {undeclared:?}\nexported: {exported:?}"
    );
}
