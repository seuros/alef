use super::{
    WasmBackend, cargo::gen_cargo_toml, fix_dropped_payload_enum_option_fields,
    types_needing_self_delegation_reverse_impl,
};
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

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
fn cargo_toml_declares_explicit_features_as_passthrough_without_enabling_default() {
    // binding-side `#[cfg(feature = X)]` items intentionally remain hidden
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
        !cargo_toml.contains("default = ["),
        "no default = [...] line — binding-side cfg items stay hidden:\n{cargo_toml}"
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
