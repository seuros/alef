//! Regression coverage for `#[serde(untagged)]` data enums in the WASM backend.
//!
//! A payload-carrying untagged enum (e.g. `enum EmbeddingInput { Single(String),
//! Multiple(Vec<String>) }`) cannot be represented as a fieldless `#[wasm_bindgen]` C-style enum
//! without discarding every variant's data. These tests assert the exact emitted Rust for a
//! struct field of that type — before the fix, `gen_enum` silently degraded it to
//! `pub enum WasmEmbeddingInput { Single = 0, Multiple = 1 }` and the containing struct's setter
//! accepted that fieldless enum, so no JS caller could ever supply the payload.

use super::{WasmBackend, enums::is_untagged_data_enum};
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

/// A free function taking `type_name` by value, so `input_type_names` (see
/// `crate::codegen::conversions`) treats it as an input type and emits the binding->core `From`
/// impl the test asserts on — without a caller, that impl is dead code the generator skips.
fn function_taking(type_name: &str) -> FunctionDef {
    FunctionDef {
        name: format!("use_{}", type_name.to_lowercase()),
        rust_path: format!("test_lib::use_{}", type_name.to_lowercase()),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::Named(type_name.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

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

/// `enum EmbeddingInput { Single(String), Multiple(Vec<String>) }` with `#[serde(untagged)]`.
/// Covers both a scalar-payload variant and a `Vec`-payload variant in one enum, which is the
/// shape that used to collapse to a bare discriminant.
fn embedding_input_enum() -> EnumDef {
    EnumDef {
        name: "EmbeddingInput".to_string(),
        rust_path: "test_lib::EmbeddingInput".to_string(),
        variants: vec![
            EnumVariant {
                name: "Single".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Multiple".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            },
        ],
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        ..Default::default()
    }
}

#[test]
fn is_untagged_data_enum_true_for_payload_carrying_untagged_enum() {
    assert!(is_untagged_data_enum(&embedding_input_enum()));
}

#[test]
fn is_untagged_data_enum_false_for_fieldless_untagged_enum() {
    let mut e = embedding_input_enum();
    for variant in &mut e.variants {
        variant.fields.clear();
        variant.is_tuple = false;
    }
    assert!(
        !is_untagged_data_enum(&e),
        "an untagged enum with only unit variants has nothing to lose and must keep the old \
         fieldless C-style representation"
    );
}

#[test]
fn is_untagged_data_enum_false_for_internally_tagged_data_enum() {
    let mut e = embedding_input_enum();
    e.serde_untagged = false;
    e.serde_tag = Some("type".to_string());
    assert!(
        !is_untagged_data_enum(&e),
        "internally-tagged data enums take the discriminator-struct path, not the JsValue-field \
         path — the two predicates must stay mutually exclusive"
    );
}

/// A struct with a *required* field of the untagged data enum type, mirroring
/// `EmbeddingRequest { pub input: EmbeddingInput, .. }`.
fn embedding_request_type() -> TypeDef {
    TypeDef {
        name: "EmbeddingRequest".to_string(),
        rust_path: "test_lib::EmbeddingRequest".to_string(),
        fields: vec![FieldDef {
            name: "input".to_string(),
            ty: TypeRef::Named("EmbeddingInput".to_string()),
            optional: false,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }
}

#[test]
fn required_untagged_data_enum_field_becomes_js_value_not_fieldless_wasm_enum() {
    let mut api = empty_api();
    api.enums = vec![embedding_input_enum()];
    api.types = vec![embedding_request_type()];
    api.functions = vec![function_taking("EmbeddingRequest")];

    let config = make_config();
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        !lib_rs.contains("pub enum WasmEmbeddingInput"),
        "no fieldless discriminant enum must be emitted for a payload-carrying untagged enum — \
         it can never carry the variant's data;\nactual:\n{lib_rs}"
    );

    assert!(
        lib_rs.contains("input: JsValue,"),
        "the struct field must be stored as JsValue so the payload round-trips;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn set_input(&mut self, value: JsValue)"),
        "the setter must accept JsValue (any JS value), not the old fieldless enum;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn input(&self) -> JsValue"),
        "the getter must return JsValue, not a wire string of the variant name;\nactual:\n{lib_rs}"
    );

    assert!(
        lib_rs.contains("input: serde_wasm_bindgen::to_value(&val.input).unwrap_or(JsValue::NULL)"),
        "core->binding conversion must serialize the real enum value via serde_wasm_bindgen;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("input: serde_wasm_bindgen::from_value(val.input.clone()).unwrap_or_default()"),
        "binding->core conversion must deserialize the JsValue back into the real enum via \
         serde_wasm_bindgen;\nactual:\n{lib_rs}"
    );
}

/// Same shape as above but the field is `Option<EmbeddingInput>` — must degrade to
/// `Option<JsValue>` throughout, not `JsValue` alone (which would make `None` unrepresentable)
/// nor the old fieldless enum.
#[test]
fn optional_untagged_data_enum_field_becomes_option_js_value() {
    let mut api = empty_api();
    api.enums = vec![embedding_input_enum()];
    api.functions = vec![function_taking("ModerationRequest")];
    api.types = vec![TypeDef {
        name: "ModerationRequest".to_string(),
        rust_path: "test_lib::ModerationRequest".to_string(),
        fields: vec![FieldDef {
            name: "input".to_string(),
            ty: TypeRef::Named("EmbeddingInput".to_string()),
            optional: true,
            ..Default::default()
        }],
        has_serde: true,
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
        !lib_rs.contains("pub enum WasmEmbeddingInput"),
        "no fieldless discriminant enum must be emitted;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("input: Option<JsValue>,"),
        "an optional field of this type must be Option<JsValue>;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn set_input(&mut self, value: Option<JsValue>)"),
        "the setter must accept Option<JsValue>;\nactual:\n{lib_rs}"
    );
}

/// A genuinely fieldless enum (no `#[serde(untagged)]`, no data variants) used as a struct field
/// must be entirely unaffected by this fix: it keeps the `Wasm{Enum}` C-style representation,
/// the `to_api_str`/`from_api_str` wire-string getter/setter, and its own conversions.
#[test]
fn fieldless_enum_field_is_unaffected() {
    let mut api = empty_api();
    api.enums = vec![EnumDef {
        name: "Role".to_string(),
        rust_path: "test_lib::Role".to_string(),
        variants: vec![
            EnumVariant {
                name: "User".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Assistant".to_string(),
                ..Default::default()
            },
        ],
        has_serde: true,
        is_copy: true,
        ..Default::default()
    }];
    api.functions = vec![function_taking("Message")];
    api.types = vec![TypeDef {
        name: "Message".to_string(),
        rust_path: "test_lib::Message".to_string(),
        fields: vec![FieldDef {
            name: "role".to_string(),
            ty: TypeRef::Named("Role".to_string()),
            optional: false,
            ..Default::default()
        }],
        has_serde: true,
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
        lib_rs.contains("pub enum WasmRole {"),
        "a genuinely fieldless enum must keep its wasm-bindgen C-style representation;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("role: WasmRole,"),
        "a field of a genuinely fieldless enum must keep the WasmRole wrapper type, not JsValue;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn role(&self) -> String"),
        "the getter for a genuinely fieldless enum field must be unchanged (wire-string via \
         to_api_str);\nactual:\n{lib_rs}"
    );
}

fn make_config_with_text_types(text_types: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
untagged_union_text_types = [{text_types}]
[crates.wasm]
"#
    ))
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// An untagged data enum that is *also* opted into `untagged_union_text_types` used to be
/// generated two different ways at once: the `type_overrides` entry pinned to `String` drove the
/// constructor, getter, and setter, while the JsValue-bridged set drove the struct field and both
/// conversions. The emitted struct declared `Option<JsValue>` and handed it to accessors typed
/// `Option<String>`, so the whole binding crate failed to compile with E0308. The text opt-in is
/// the more specific signal and must win on every surface.
#[test]
fn untagged_data_enum_in_text_types_is_string_on_every_surface() {
    let mut api = empty_api();
    api.enums = vec![embedding_input_enum()];
    api.functions = vec![function_taking("ModerationRequest")];
    api.types = vec![TypeDef {
        name: "ModerationRequest".to_string(),
        rust_path: "test_lib::ModerationRequest".to_string(),
        fields: vec![FieldDef {
            name: "input".to_string(),
            ty: TypeRef::Named("EmbeddingInput".to_string()),
            optional: true,
            ..Default::default()
        }],
        has_serde: true,
        ..Default::default()
    }];

    let config = make_config_with_text_types("\"EmbeddingInput\"");
    let files = WasmBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content;

    assert!(
        lib_rs.contains("input: Option<String>,"),
        "the struct field must follow the text opt-in, not the JsValue bridge;\nactual:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("input: Option<JsValue>,"),
        "the JsValue-bridged representation must not be emitted for a text-typed union;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn input(&self) -> Option<String>"),
        "the getter must agree with the field type;\nactual:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub fn set_input(&mut self, value: Option<String>)"),
        "the setter must agree with the field type;\nactual:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("serde_wasm_bindgen::to_value(&val.input)"),
        "conversions must use the display-text bridge, not serde_wasm_bindgen;\nactual:\n{lib_rs}"
    );
}
