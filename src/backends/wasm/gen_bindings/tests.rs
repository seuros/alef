use super::{
    WasmBackend, cargo::gen_cargo_toml, fix_dropped_payload_enum_option_fields, forward_trait_bridge_builder_fields,
    WasmCallability, function_is_exported, types_needing_self_delegation_reverse_impl, wasm_callability,
};
use crate::core::backend::Backend;
use crate::core::config::{BridgeBinding, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{
    ApiSurface, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn make_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

#[test]
fn wasm_backend_name_is_wasm() {
    assert_eq!(WasmBackend.name(), "wasm");
}

#[test]
fn generate_bindings_empty_api_produces_files() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].path.to_string_lossy().ends_with("lib.rs"));
    assert!(files[1].path.to_string_lossy().ends_with("Cargo.toml"));
}

#[test]
fn extra_dependency_overrides_builtin_without_duplicate_key() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dependencies]
serde = { version = "1", features = ["derive", "rc"] }
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);

    let serde_lines = cargo_toml
        .lines()
        .filter(|l| l.trim_start().starts_with("serde =") || l.trim_start().starts_with("serde="))
        .count();
    assert_eq!(serde_lines, 1, "expected exactly one `serde` key, got:\n{cargo_toml}");
    assert!(
        cargo_toml.contains(r#"features = ["derive", "rc"]"#),
        "extra_dependencies override should win:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

/// `[crates.cargo_lints]` must round-trip into the emitted wasm `Cargo.toml` as a
/// `[lints.rust]` / `[lints.clippy]` block, and produce valid TOML. The wasm crate
/// has no builtin `[lints.*]` block of its own, so this is a plain splice, not a merge.
#[test]
fn cargo_toml_emits_configured_cargo_lints() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.cargo_lints.rust]
unused_must_use = "deny"

[crates.cargo_lints.clippy]
print_stdout = "deny"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let cargo_toml = gen_cargo_toml(&empty_api(), &config);

    assert!(
        cargo_toml.contains("[lints.rust]\nunused_must_use = \"deny\""),
        "expected [lints.rust] block, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("[lints.clippy]\nprint_stdout = \"deny\""),
        "expected [lints.clippy] block, got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml with cargo_lints must be valid TOML");
}

/// Absence of `[crates.cargo_lints]` must not emit any `[lints]` table at all.
#[test]
fn cargo_toml_omits_lints_block_when_cargo_lints_unset() {
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&empty_api(), &config);
    assert!(
        !cargo_toml.contains("[lints"),
        "no [lints] table should be emitted when cargo_lints is unset, got:\n{cargo_toml}"
    );
}

#[test]
fn cargo_toml_emits_passthrough_features_for_type_cfg_attrs() {
    use crate::core::ir::TypeDef;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "PdfThing".to_string(),
            rust_path: "test_lib::PdfThing".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        cargo_toml.contains(r#"pdf = ["test-lib/pdf"]"#),
        "expected `pdf = [\"test-lib/pdf\"]` in:\n{cargo_toml}"
    );
    assert_eq!(
        cargo_toml.matches("\n[features]\n").count(),
        1,
        "exactly one [features] block expected:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);
    assert!(
        !cargo_toml.contains("[features]"),
        "expected no [features] block:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_declares_configured_extra_features_without_enabling_them() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
extra_features = ["sceptre-wasm", "", "sceptre-wasm", "telemetry"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert_eq!(
        cargo_toml
            .matches(r#"sceptre-wasm = ["test-lib/sceptre-wasm"]"#)
            .count(),
        1,
        "extra features must be deduplicated in:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"telemetry = ["test-lib/telemetry"]"#),
        "expected telemetry passthrough in:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("default = ["),
        "extra features must remain opt-in in:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_enables_configured_binding_features_by_default() {
    use crate::core::ir::TypeDef;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
features = ["wasm-target"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GatedType".to_string(),
            rust_path: "test_lib::GatedType".to_string(),
            cfg: Some(r#"any(feature = "wasm-target", feature = "extra")"#.to_string()),
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let cargo_toml = gen_cargo_toml(&api, &config);
    assert!(
        cargo_toml.contains(r#"extra = ["test-lib/extra"]"#),
        "expected `extra` passthrough:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"wasm-target = ["test-lib/wasm-target"]"#),
        "wasm-target must be declared as passthrough so rustc sees the feature:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"default = ["wasm-target"]"#),
        "configured core features must also enable matching binding-side cfg gates:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains(r#"default = ["extra"]"#),
        "unconfigured discovered gates must remain opt-in:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn cargo_toml_has_no_issues_docs_line_and_getrandom_deps_are_alphabetical() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config();
    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        !cargo_toml.contains("Issues & docs:"),
        "Cargo.toml must not contain 'Issues & docs:' line — cargo-sort strips it and \
             alef re-emits it, causing prek to loop forever:\n{cargo_toml}"
    );

    let pos_02 = cargo_toml
        .find("getrandom_02")
        .expect("getrandom_02 must be present in target deps");
    let pos_03 = cargo_toml
        .find("getrandom_03")
        .expect("getrandom_03 must be present in target deps");
    assert!(
        pos_02 < pos_03,
        "getrandom_02 must appear before getrandom_03 (alphabetical order for cargo-sort \
             compatibility); got getrandom_02 at {pos_02}, getrandom_03 at {pos_03}:\n{cargo_toml}"
    );

    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn test_visitor_field_substitution_in_post_process() {
    let mut content = "impl From<WasmConversionOptions> for sample_markup_rs::options::ConversionOptions {\n    fn from(val: WasmConversionOptions) -> Self {\n        Self {\n            heading_style: val.heading_style.into(),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\nimpl From<WasmConversionOptionsUpdate> for sample_markup_rs::options::ConversionOptionsUpdate {\n    fn from(val: WasmConversionOptionsUpdate) -> Self {\n        Self {\n            heading_style: val.heading_style.map(Into::into),\n            visitor: Default::default(),\n            ..Default::default()\n        }\n    }\n}\n".to_string();

    let field_name = "visitor";
    let patterns = &[
        ("            ", "\n            "),
        ("        ", "\n        "),
        ("  ", "\n  "),
    ];
    for (indent, newline_indent) in patterns {
        let old_pattern = format!("{indent}{field_name}: Default::default(),{newline_indent}..Default::default()");
        let new_pattern = format!(
            "{indent}{field_name}: val.{field_name}.map(|v| (*v.inner).clone()),{newline_indent}..Default::default()"
        );
        if content.contains(&old_pattern) {
            content = content.replace(&old_pattern, &new_pattern);
        }
    }

    assert!(
        content.contains("visitor: val.visitor.map(|v| (*v.inner).clone()),"),
        "Visitor field not forwarded in From impl"
    );
    assert!(
        !content.contains("visitor: Default::default(),\n            ..Default::default()"),
        "Unreplaced visitor: Default::default() with 12 spaces still present"
    );
}

#[test]
fn trait_bridge_builder_field_forwards_the_handle() {
    let bridge = TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        param_name: Some("renderer".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("renderer".to_string()),
        ..Default::default()
    };
    let content = "core_options.renderer(renderer.as_ref().map(|v| &v.inner));".to_string();

    let generated = forward_trait_bridge_builder_fields(content, &[bridge]);

    assert_eq!(
        generated,
        "core_options.renderer(renderer.map(|v| (*v.inner).clone()));"
    );
    assert!(!generated.contains(".renderer(None)"));
}

#[test]
fn cargo_toml_emits_extra_dev_dependencies() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dev_dependencies]
wasm-bindgen-test = "0.3"
serde_json = { version = "1", features = ["preserve_order"] }
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let cargo_toml = gen_cargo_toml(&api, &config);

    assert!(
        cargo_toml.contains("[dev-dependencies]"),
        "expected a [dev-dependencies] section in:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"wasm-bindgen-test = "0.3""#),
        "expected the string-valued dev dependency in:\n{cargo_toml}"
    );
    let parsed: toml::Value = toml::from_str(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
    let dev = parsed
        .get("dev-dependencies")
        .expect("dev-dependencies table must exist");
    assert!(dev.get("serde_json").and_then(|v| v.get("features")).is_some());

    let plain = gen_cargo_toml(&api, &make_config());
    assert!(
        !plain.contains("[dev-dependencies]"),
        "unexpected dev-deps in:\n{plain}"
    );
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders manifest
/// tables `[dependencies]` -> `[target.'cfg(...)'.dependencies]` ->
/// `[build-dependencies]` -> `[dev-dependencies]`. The wasm binding crate
/// always carries a `[target.'cfg(target_arch = "wasm32")'.dependencies]`
/// block for `getrandom`, so whenever `extra_dev_dependencies` also produces a
/// `[dev-dependencies]` section, the target block must come first — cargo-sort
/// rejects a manifest with `[dev-dependencies]` before a later `[target.*]`
/// table.
#[test]
fn cargo_toml_orders_target_block_before_dev_dependencies() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
[crates.wasm.extra_dev_dependencies]
wasm-bindgen-test = "0.3"
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let cargo_toml = gen_cargo_toml(&api, &config);

    let target_pos = cargo_toml
        .find("[target.'cfg(target_arch = \"wasm32\")'.dependencies]")
        .expect("expected the wasm32 target block");
    let dev_pos = cargo_toml
        .find("[dev-dependencies]")
        .expect("expected a [dev-dependencies] section");

    assert!(
        target_pos < dev_pos,
        "the [target.*] block must precede [dev-dependencies]; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated Cargo.toml must be valid TOML");
}

/// Regression test for a wasm-only E0308: a type that is never a function/method *parameter*
/// (directly or transitively) has no reason to appear in `input_type_names`, so the
/// binding->core `From` impl is normally skipped for it. But if that same type has an
/// auto-delegated instance method (e.g. `PageRange::page_count(&self) -> u32`, only ever
/// *returned*, never taken as input), `gen_method` still emits
/// `{core}::{Type}::from(self.clone()).{method}(..)`, which requires exactly that impl to exist.
/// `types_needing_self_delegation_reverse_impl` must flag such types so the reverse impl gets
/// generated regardless of `input_type_names`.
#[test]
fn types_needing_self_delegation_reverse_impl_flags_return_only_delegating_type() {
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "PageRange".to_string(),
        rust_path: "test_lib::PageRange".to_string(),
        fields: vec![
            FieldDef {
                name: "start".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
            FieldDef {
                name: "end".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
        ],
        methods: vec![MethodDef {
            name: "page_count".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let needed = types_needing_self_delegation_reverse_impl(&api, &ahash::AHashSet::default());
    assert!(
        needed.contains("PageRange"),
        "a type with a self-delegating instance method must require the binding->core reverse \
         impl even though it is never used as an input, got {needed:?}"
    );
}

/// A type none of whose methods reach the self-delegation branch must NOT be flagged — doing so
/// would only add dead, unused `From` impls.
///
/// Note the `&mut self` method has to be non-delegatable for this to hold. `gen_method` routes an
/// opaque type's *non*-mut methods through the mutex-lock path
/// (`self.inner.lock().unwrap().{method}(..)`, methods.rs:156), but its `&mut self` methods fall
/// through to the `self.clone()` self-delegation form — so an opaque type with any delegatable
/// `&mut self` method genuinely does need the reverse impl. `sanitized` is what makes `resize`
/// non-delegatable here.
#[test]
fn types_needing_self_delegation_reverse_impl_ignores_opaque_mutex_delegated_type() {
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "Pool".to_string(),
        rust_path: "test_lib::Pool".to_string(),
        is_opaque: true,
        methods: vec![
            MethodDef {
                name: "resize".to_string(),
                return_type: TypeRef::Primitive(PrimitiveType::Bool),
                receiver: Some(ReceiverKind::RefMut),
                sanitized: true,
                ..Default::default()
            },
            MethodDef {
                name: "len".to_string(),
                return_type: TypeRef::Primitive(PrimitiveType::Usize),
                receiver: Some(ReceiverKind::Ref),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let opaque_types: ahash::AHashSet<String> = ["Pool".to_string()].into_iter().collect();

    let needed = types_needing_self_delegation_reverse_impl(&api, &opaque_types);
    assert!(
        !needed.contains("Pool"),
        "an opaque type whose non-mut methods route through the mutex-lock path needs no \
         binding->core reverse impl, got {needed:?}"
    );
}

/// End-to-end coverage: a type that is only ever returned (never an input) but has an
/// auto-delegated instance method must get a real `impl From<Wasm{T}> for {core}::{T}` in the
/// actual generated `lib.rs`, and `gen_method`'s self-delegation call must reference that exact
/// core type -- a real downstream wasm crate failed to compile with E0308 before this fix.
#[test]
fn generated_lib_rs_has_reverse_impl_for_return_only_delegating_type() {
    let mut api = empty_api();
    api.types = vec![TypeDef {
        name: "PageRange".to_string(),
        rust_path: "test_lib::PageRange".to_string(),
        fields: vec![
            FieldDef {
                name: "start".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
            FieldDef {
                name: "end".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            },
        ],
        methods: vec![MethodDef {
            name: "page_count".to_string(),
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        lib_rs.contains("impl From<WasmPageRange> for test_lib::PageRange {"),
        "expected a binding->core reverse impl for PageRange:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("test_lib::PageRange::from(self.clone()).page_count()"),
        "expected the self-delegation call the reverse impl above exists to support:\n{lib_rs}"
    );
}

#[test]
fn instance_method_with_borrowed_named_input_delegates_to_core() {
    let method = MethodDef {
        name: "evaluate".to_string(),
        params: vec![ParamDef {
            name: "options".to_string(),
            ty: TypeRef::Named("Options".to_string()),
            is_ref: true,
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        error_type: Some("EvaluationError".to_string()),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Evaluator".to_string(),
        methods: vec![method.clone()],
        ..Default::default()
    };
    let mapper = crate::backends::wasm::type_map::WasmMapper::new(Default::default(), "Wasm".to_string());
    let output = super::methods::gen_method(
        &method,
        &mapper,
        "Evaluator",
        "sample_core",
        &Default::default(),
        "Wasm",
        &typ,
        &Default::default(),
        &Default::default(),
    );

    assert!(output.contains("let options_core: sample_core::Options"), "{output}");
    assert!(
        output.contains("sample_core::Evaluator::from(self.clone()).evaluate(&options_core)"),
        "{output}"
    );
    assert!(!output.contains("Not implemented"), "{output}");
}

/// Regression test for a wasm-only E0282: a field whose Rust type is a payload-carrying enum
/// (`#[serde(tag = "type")]` with struct variants) has no wasm-bindgen representation, so
/// `gen_struct` drops it from the generated Wasm struct. The shared binding->core `From`
/// conversion generator does not know that, and for an `Option<Box<T>>` field falls back to
/// `Default::default().map(Box::new)` -- untypeable, since nothing pins down `T`. The post-process
/// fixup must replace it with a self-documenting `None`.
#[test]
fn fix_dropped_payload_enum_option_fields_replaces_untypeable_default_with_documented_none() {
    let content = "impl From<test_lib::LlmConfig> for WasmLlmConfig {\n    fn from(val: test_lib::LlmConfig) -> Self {\n        Self {\n            model: val.model,\n        }\n    }\n}\nimpl From<WasmLlmConfig> for test_lib::LlmConfig {\n    fn from(val: WasmLlmConfig) -> Self {\n        Self {\n            model: val.model,\n            credential_provider: Default::default().map(Box::new),\n        }\n    }\n}\n".to_string();

    let fixed = fix_dropped_payload_enum_option_fields(content);

    assert!(
        !fixed.contains("Default::default().map(Box::new)"),
        "untypeable expression must be fully replaced:\n{fixed}"
    );
    assert!(
        fixed.contains("credential_provider: None,"),
        "field must fall back to a literal `None`:\n{fixed}"
    );
    assert!(
        fixed.contains("// ALEF-OMITTED: `credential_provider` is always None on wasm"),
        "the omission must be documented in the generated source so a reader learns why the \
         field is always None:\n{fixed}"
    );
}

/// The fixup must be a no-op on content that never had the buggy pattern -- it must not, for
/// example, touch ordinary `field: Default::default(),` lines that don't end in `.map(Box::new)`.
#[test]
fn fix_dropped_payload_enum_option_fields_is_noop_without_the_pattern() {
    let content = "Self {\n    reason: ChunkingReason::default(),\n    other: Default::default(),\n}\n".to_string();
    let fixed = fix_dropped_payload_enum_option_fields(content.clone());
    assert_eq!(fixed, content, "content without the buggy pattern must be unchanged");
}

#[test]
fn wasm_function_reachability_follows_target_features() {
    let functions = vec![
        FunctionDef {
            name: "download".into(),
            rust_path: "sample::download".into(),
            cfg: Some(r#"feature = "download""#.into()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "prefetch".into(),
            rust_path: "sample::prefetch".into(),
            cfg: Some(r#"not(feature = "download")"#.into()),
            ..FunctionDef::default()
        },
    ];
    let config = make_config();

    assert!(!function_is_exported("download", &functions, &config));
    assert!(function_is_exported("prefetch", &functions, &config));
}

fn reachability_functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef {
            name: "download_assets".into(),
            rust_path: "sample::download_assets".into(),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "gated_download".into(),
            rust_path: "sample::gated_download".into(),
            cfg: Some(r#"feature = "download""#.into()),
            ..FunctionDef::default()
        },
    ]
}

#[test]
fn wasm_callability_accepts_the_javascript_spelling_of_an_exported_function() {
    let functions = reachability_functions();
    let config = make_config();

    assert_eq!(
        wasm_callability("downloadAssets", &functions, &config),
        WasmCallability::Callable,
        "`overrides.wasm.function` names the symbol the way wasm-bindgen exports it"
    );
    assert_eq!(
        wasm_callability("download_assets", &functions, &config),
        WasmCallability::Callable,
        "the Rust spelling must keep working for calls that carry no override"
    );
}

#[test]
fn wasm_callability_accepts_a_bridge_registry_operation_under_either_spelling() {
    let functions = reachability_functions();
    let mut config = make_config();
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "RerankerBackend".into(),
        clear_fn: Some("clear_reranker_backends".into()),
        unregister_fn: Some("unregister_reranker_backend".into()),
        ..Default::default()
    }];

    assert_eq!(
        wasm_callability("clearRerankerBackends", &functions, &config),
        WasmCallability::Callable
    );
    assert_eq!(
        wasm_callability("unregister_reranker_backend", &functions, &config),
        WasmCallability::Callable
    );
    assert!(
        !function_is_exported("clear_reranker_backends", &functions, &config),
        "the codegen predicate must keep answering `false` -- the plain function generator does \
         not emit bridge-managed functions, the trait-bridge generator does"
    );
}

#[test]
fn wasm_callability_tells_an_unknown_name_apart_from_an_unexported_one() {
    let functions = reachability_functions();
    let config = make_config();

    assert_eq!(
        wasm_callability("gatedDownload", &functions, &config),
        WasmCallability::NotExported,
        "a real function the target drops is a capability gap"
    );
    assert_eq!(
        wasm_callability("fetchAssets", &functions, &config),
        WasmCallability::UnknownSymbol,
        "a name nothing answers to is a config error and must not be reported as a capability gap"
    );
    assert_eq!(
        wasm_callability("", &functions, &config),
        WasmCallability::UnknownSymbol,
        "an unresolved name must never be answered with a confident `not exported`"
    );
}

#[test]
fn wasm_callability_honours_an_exclusion_reached_by_the_javascript_spelling() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.wasm]
exclude_functions = ["download_assets"]
"#,
    )
    .expect("an exclusion list must deserialize");
    let config = cfg.resolve().expect("config must resolve").remove(0);

    assert_eq!(
        wasm_callability("downloadAssets", &reachability_functions(), &config),
        WasmCallability::NotExported,
        "resolving the JavaScript spelling must not route around `exclude_functions`"
    );
}

/// Regression test: wasm-pack's own `pkg/nodejs/package.json` (produced by
/// `--target nodejs --out-dir pkg/nodejs`) declares a `"name"` derived from the wasm crate's
/// `Cargo.toml`, not `config.wasm_package_name()` — the name every e2e-generated `file:`
/// dependency and `require()`/`import` specifier actually uses. Without a post-build step to
/// reconcile them, the specifier names a package the built directory does not declare.
#[test]
fn build_config_with_config_rewrites_wasm_pack_package_json_name() {
    let backend = WasmBackend;
    let config = make_config();

    let build_config = backend
        .build_config_with_config(&config)
        .expect("wasm backend must report a build config");

    let rewrite_step = build_config
        .post_build
        .iter()
        .find_map(|step| match step {
            crate::core::backend::PostBuildStep::RewriteWasmPackageName {
                package_json_path,
                package_name,
            } => Some((package_json_path.clone(), package_name.clone())),
            _ => None,
        })
        .expect("build_config_with_config must attach a RewriteWasmPackageName post-build step");

    assert_eq!(rewrite_step.1, "test-lib-wasm");
    assert_eq!(
        rewrite_step.0,
        std::path::PathBuf::from("crates/test-lib-wasm/pkg/nodejs/package.json"),
        "the rewrite target must be the exact directory `build_command_for`'s wasm-pack \
         arm builds into: {:?}",
        rewrite_step.0
    );
}

fn rewrite_target_for(toml_src: &str) -> std::path::PathBuf {
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    let config = cfg.resolve().unwrap().remove(0);

    WasmBackend
        .build_config_with_config(&config)
        .expect("wasm backend must report a build config")
        .post_build
        .iter()
        .find_map(|step| match step {
            crate::core::backend::PostBuildStep::RewriteWasmPackageName { package_json_path, .. } => {
                Some(package_json_path.clone())
            }
            _ => None,
        })
        .expect("build_config_with_config must attach a RewriteWasmPackageName post-build step")
}

/// Failure path: when `[crates.output] wasm` is set, `build_command_for` resolves the crate
/// dir from *that* path (dropping a trailing `src`, which holds the generated sources rather
/// than the crate root) and ignores the `package_dir` default formula. This step must follow
/// the same rule, or the build writes `pkg/nodejs` under one directory while the rewrite
/// looks under another — and since a missing file is only debug-logged, the rewrite would
/// silently never fire and the name mismatch would survive the build. The two directories
/// below deliberately disagree, which is exactly what a `package_dir`-only derivation gets
/// wrong. ~keep
#[test]
fn rewrite_target_follows_explicit_output_rather_than_the_package_dir_formula() {
    let target = rewrite_target_for(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.output]
wasm = "crates/renamed-wasm-crate/src/"
"#,
    );

    assert_eq!(
        target,
        std::path::PathBuf::from("crates/renamed-wasm-crate/pkg/nodejs/package.json")
    );
    assert!(
        !target.starts_with("crates/test-lib-wasm"),
        "must not fall back to the package_dir formula when [crates.output] wasm is set: {target:?}"
    );
}

/// An explicit output that already names the crate root (no trailing `src`) must be used
/// verbatim rather than having its last component stripped.
#[test]
fn rewrite_target_keeps_an_explicit_output_that_is_already_the_crate_root() {
    let target = rewrite_target_for(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.output]
wasm = "crates/renamed-wasm-crate"
"#,
    );

    assert_eq!(
        target,
        std::path::PathBuf::from("crates/renamed-wasm-crate/pkg/nodejs/package.json")
    );
}

/// Async instance methods return `{mapped}::from(result)`, which only compiles when the mapped
/// type has a `From<CoreType>`. The wasm mapper collapses every `Map` (and every `Json`, and any
/// `Named` a `type_overrides` entry redirects) onto the opaque `JsValue`, which has no such impl
/// — `JsValue::from(HashMap<..>)` is an `E0277`. The value must cross through serde instead,
/// exactly as the generated `From<CoreType> for WasmType` bodies do for degraded fields.
#[test]
fn async_method_returning_map_bridges_through_serde_not_from() {
    let method = MethodDef {
        name: "headers".to_string(),
        is_async: true,
        return_type: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Request".to_string(),
        methods: vec![method.clone()],
        ..Default::default()
    };
    let mapper = crate::backends::wasm::type_map::WasmMapper::new(Default::default(), "Wasm".to_string());

    let output = super::methods::gen_method(
        &method,
        &mapper,
        "Request",
        "sample_core",
        &Default::default(),
        "Wasm",
        &typ,
        &Default::default(),
        &Default::default(),
    );

    assert!(
        output.contains("serde_wasm_bindgen::to_value(&result)"),
        "a JsValue-mapped return must be serialized:\n{output}"
    );
    assert!(
        !output.contains("JsValue::from(result)"),
        "JsValue has no From<HashMap<..>>:\n{output}"
    );
}

/// Positive control for the above: a `Named` return really does map to a generated wrapper with
/// a `From<CoreType>`, so the turbofish `from` must stay.
#[test]
fn async_method_returning_named_still_uses_from() {
    let method = MethodDef {
        name: "report".to_string(),
        is_async: true,
        return_type: TypeRef::Named("Report".to_string()),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Request".to_string(),
        methods: vec![method.clone()],
        ..Default::default()
    };
    let mapper = crate::backends::wasm::type_map::WasmMapper::new(Default::default(), "Wasm".to_string());

    let output = super::methods::gen_method(
        &method,
        &mapper,
        "Request",
        "sample_core",
        &Default::default(),
        "Wasm",
        &typ,
        &Default::default(),
        &Default::default(),
    );

    assert!(
        output.contains("WasmReport::from(result)"),
        "a wrapper-mapped return must keep the direct From conversion:\n{output}"
    );
    assert!(
        !output.contains("serde_wasm_bindgen::to_value(&result)"),
        "a wrapper-mapped return must not detour through serde:\n{output}"
    );
}
