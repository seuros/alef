//! Every `NativeMethods.X(...)` the C# backend emits must have a `[DllImport]` declaration in the
//! same run's `NativeMethods.cs`.
//!
//! A name used in a `.jinja` template and declared nowhere is `CS0117` — the generated package
//! does not compile at all, no matter how correct every signature around it is. Nothing else in
//! this repo cross-checks the two sides: the emitters that write the calls and the emitters that
//! write the declarations live in different modules, are gated on different predicates, and are
//! covered by tests that only ever look at one side.
//!
//! The fixture below deliberately exercises a `bind_via = "options_field"` trait bridge, whose
//! wrapper is the only shape that reaches `bridge_field_register.jinja` /
//! `bridge_field_unregister.jinja` / `bridge_field_inject.jinja`. Those three templates called
//! `{Trait}BridgeNew`, `{Trait}BridgeFree` and `{Options}Set{Field}` with no declaration behind
//! them, which is what this file exists to keep fixed. ~keep

use alef::backends::csharp::CsharpBackend;
use alef::backends::ffi::trait_bridge::{bridge_new_free_symbols, gen_bridge_new_free};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, FieldDef, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::BTreeSet;

const CRATE_NAME: &str = "sample_crate";
const TRAIT_NAME: &str = "NodeVisitor";
const OPTIONS_TYPE: &str = "RenderOptions";
const OPTIONS_FIELD: &str = "visitor";

/// A trait bridge bound to a field of an options struct, plus the one free function that takes
/// that struct — the minimum shape that makes the C# backend emit a bridge-field wrapper.
fn surface() -> ApiSurface {
    let visit = MethodDef {
        name: "visit_node".to_owned(),
        params: vec![ParamDef {
            name: "node".to_owned(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Primitive(alef::core::ir::PrimitiveType::Bool),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };

    ApiSurface {
        crate_name: CRATE_NAME.to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![
            TypeDef {
                name: OPTIONS_TYPE.to_owned(),
                rust_path: format!("{CRATE_NAME}::{OPTIONS_TYPE}"),
                fields: vec![
                    FieldDef {
                        name: "width".to_owned(),
                        ty: TypeRef::Primitive(alef::core::ir::PrimitiveType::U32),
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: OPTIONS_FIELD.to_owned(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)),
                        ..FieldDef::default()
                    },
                ],
                is_clone: true,
                ..TypeDef::default()
            },
            TypeDef {
                name: TRAIT_NAME.to_owned(),
                rust_path: format!("{CRATE_NAME}::{TRAIT_NAME}"),
                methods: vec![visit],
                is_trait: true,
                ..TypeDef::default()
            },
        ],
        functions: vec![FunctionDef {
            name: "render".to_owned(),
            rust_path: format!("{CRATE_NAME}::render"),
            params: vec![ParamDef {
                name: "options".to_owned(),
                ty: TypeRef::Named(OPTIONS_TYPE.to_owned()),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    }
}

/// `[ffi]` is left unset, so `visitor_callbacks` reads as `false`. That is the branch in which
/// nothing else declares the `{Options}Set{Field}` setter, even though the FFI crate exports it
/// for every options-field bridge. ~keep
fn config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: CRATE_NAME.to_owned(),
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: TRAIT_NAME.to_owned(),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some(OPTIONS_TYPE.to_owned()),
            options_field: Some(OPTIONS_FIELD.to_owned()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn generate() -> Vec<GeneratedFile> {
    CsharpBackend
        .generate_bindings(&surface(), &config())
        .expect("generation should succeed")
}

fn csharp_files(files: &[GeneratedFile]) -> Vec<(&str, &str)> {
    files
        .iter()
        .filter_map(|generated| {
            let name = generated.path.file_name()?.to_str()?;
            name.ends_with(".cs").then_some((name, generated.content.as_str()))
        })
        .collect()
}

/// Identifier immediately preceding `(`, reading right to left from `text`.
fn leading_identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// Every member name declared on `NativeMethods` that a `NativeMethods.X(...)` expression can
/// legally resolve to: the `[DllImport]` externs and the marshalled callback delegate types.
fn declared_members(files: &[(&str, &str)]) -> BTreeSet<String> {
    let mut declared = BTreeSet::new();
    for (_, content) in files {
        for line in content.lines() {
            let trimmed = line.trim_start();
            let rest = trimmed
                .strip_prefix("internal static extern ")
                .or_else(|| trimmed.strip_prefix("public delegate "))
                .or_else(|| trimmed.strip_prefix("internal delegate "));
            let Some(rest) = rest else { continue };
            let Some((_return_type, after_type)) = rest.split_once(' ') else {
                continue;
            };
            if let Some(name) = leading_identifier(after_type.trim_start()) {
                declared.insert(name.to_owned());
            }
        }
    }
    declared
}

/// Every `NativeMethods.X(` call site in the generated C#, as `(file, member)`.
fn called_members<'a>(files: &[(&'a str, &'a str)]) -> Vec<(&'a str, String)> {
    let mut calls = Vec::new();
    for (name, content) in files {
        for line in content.lines() {
            for (index, _) in line.match_indices("NativeMethods.") {
                let after = &line[index + "NativeMethods.".len()..];
                let Some(member) = leading_identifier(after) else {
                    continue;
                };
                if after[member.len()..].starts_with('(') {
                    calls.push((*name, member.to_owned()));
                }
            }
        }
    }
    calls
}

#[test]
fn every_native_method_called_by_generated_csharp_is_declared_in_native_methods() {
    let generated = generate();
    let files = csharp_files(&generated);
    let declared = declared_members(&files);
    let calls = called_members(&files);

    // Non-vacuity, asserted before the difference: a run that parsed no declarations, or that
    // never rendered the bridge templates, would satisfy an empty-difference assertion while
    // examining nothing. The three names below are exactly the call sites that had no
    // declaration, so they also pin the fixture to the shape this test was written for. ~keep
    assert!(
        declared.len() >= 4,
        "parsed {} P/Invoke declarations — the declaration scanner stopped recognising the \
         emitted shape: {declared:?}",
        declared.len()
    );
    assert!(
        calls.len() >= 5,
        "found {} `NativeMethods.*` call sites — the call scanner stopped recognising the \
         emitted shape: {calls:?}",
        calls.len()
    );
    for required in [
        "NodeVisitorBridgeNew",
        "NodeVisitorBridgeFree",
        "RenderOptionsSetVisitor",
    ] {
        assert!(
            calls.iter().any(|(_, member)| member == required),
            "the fixture must reach the options-field bridge templates, which call `{required}`; \
             without it this cross-check never examines the branch it exists for: {calls:?}"
        );
    }

    let undeclared: Vec<&(&str, String)> = calls
        .iter()
        .filter(|(_, member)| !declared.contains(member.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "generated C# calls native methods that `NativeMethods.cs` never declares — this is \
         CS0117 and the package does not compile:\n{undeclared:#?}\ndeclared: {declared:?}"
    );
}

#[test]
fn options_field_bridge_pinvokes_name_the_entry_points_the_ffi_crate_exports() {
    let generated = generate();
    let files = csharp_files(&generated);
    let native_methods = files
        .iter()
        .find(|(name, _)| *name == "NativeMethods.cs")
        .expect("NativeMethods.cs must be generated")
        .1;

    let pascal_prefix = "SampleCrate";
    let (new_fn, free_fn) = bridge_new_free_symbols(CRATE_NAME, pascal_prefix, TRAIT_NAME);
    let ffi_source = gen_bridge_new_free(CRATE_NAME, pascal_prefix, TRAIT_NAME);

    // The names are only half the contract. Assert against the FFI crate's own emitted Rust that
    // `_new` returns `AlefHandle` and `_free` consumes one, because that is what makes `ulong`
    // (not `IntPtr`, and not `int`) the correct P/Invoke spelling — a mismatched marshalling
    // width is silent memory corruption rather than a compile error. ~keep
    assert!(
        ffi_source.contains(&format!("pub unsafe extern \"C\" fn {new_fn}(")) && ffi_source.contains("-> AlefHandle {"),
        "the FFI emitter must still return `AlefHandle` from `{new_fn}`:\n{ffi_source}"
    );
    assert!(
        ffi_source.contains(&format!("pub unsafe extern \"C\" fn {free_fn}(handle: AlefHandle)")),
        "the FFI emitter must still take `AlefHandle` in `{free_fn}`:\n{ffi_source}"
    );

    for expected in [
        format!("EntryPoint = \"{new_fn}\""),
        format!("EntryPoint = \"{free_fn}\""),
        format!("EntryPoint = \"{CRATE_NAME}_options_set_{OPTIONS_FIELD}\""),
        "internal static extern ulong NodeVisitorBridgeNew(IntPtr vtable, IntPtr userData);".to_owned(),
        "internal static extern void NodeVisitorBridgeFree(ulong handle);".to_owned(),
        "internal static extern void RenderOptionsSetVisitor(ulong options, ulong bridge);".to_owned(),
    ] {
        assert!(
            native_methods.contains(&expected),
            "missing `{expected}` from the generated declarations:\n{native_methods}"
        );
    }
}
