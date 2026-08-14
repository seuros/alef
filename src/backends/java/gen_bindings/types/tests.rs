#[cfg(test)]
use super::*;
use crate::core::config::JavaBuilderMode;
use crate::core::ir::TypeDef;
use crate::core::ir::{CoreWrapper, DefaultValue, FieldDef, PrimitiveType, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

/// A plain `usize` field with a literal Rust default and NO `#[serde(default)]` — the
/// exact shape that took the zero-sentinel path.
fn make_config_type_with_primitive_default() -> TypeDef {
    let mut typ = make_config_type_with_duration_default();
    typ.fields[0].name = "max_redirects".to_string();
    typ.fields[0].ty = TypeRef::Primitive(PrimitiveType::Usize);
    typ.fields[0].default = Some("10".to_string());
    typ.fields[0].typed_default = Some(DefaultValue::IntLiteral(10));
    typ
}

fn make_config_type_with_duration_default() -> TypeDef {
    TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "sample_crate::CrawlConfig".to_string(),
        original_rust_path: "sample_crate::CrawlConfig".to_string(),
        fields: vec![FieldDef {
            version: Default::default(),
            name: "request_timeout".to_string(),
            ty: TypeRef::Duration,
            optional: false,
            default: Some("30000".to_string()),
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(DefaultValue::IntLiteral(30000)),
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_request_type_with_multiword_fields() -> TypeDef {
    TypeDef {
        name: "ChatCompletionRequest".to_string(),
        rust_path: "sample_llm::ChatCompletionRequest".to_string(),
        original_rust_path: "sample_llm::ChatCompletionRequest".to_string(),
        fields: vec![
            FieldDef {
                version: Default::default(),
                name: "model".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
                name: "max_tokens".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
                optional: true,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
                name: "top_p".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                optional: true,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// Single-word builder fields like `model` MUST get `@JsonProperty`
/// Jackson's BuilderBasedDeserializer requires @JsonProperty on every setter
/// to correctly map JSON properties to setters.
#[test]
fn single_word_builder_field_gets_json_property() {
    let typ = make_request_type_with_multiword_fields();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleLlmRs",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    assert!(
        out.contains("@JsonProperty(\"model\")"),
        "single-word builder field must get @JsonProperty; got:\n{out}"
    );
}

/// Multi-word snake_case fields like `max_tokens` → `maxTokens` MUST get
/// `@JsonProperty("max_tokens")` so Jackson sends the snake_case wire name
/// that Rust's serde expects.
#[test]
fn multiword_snake_case_field_gets_json_property_annotation() {
    let typ = make_request_type_with_multiword_fields();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleLlmRs",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    assert!(
        out.contains("@JsonProperty(\"max_tokens\")"),
        "multi-word field max_tokens must have @JsonProperty(\"max_tokens\") annotation; got:\n{out}"
    );
    assert!(
        out.contains("@JsonProperty(\"top_p\")"),
        "multi-word field top_p must have @JsonProperty(\"top_p\") annotation; got:\n{out}"
    );
    assert!(
        out.contains("import com.fasterxml.jackson.annotation.JsonProperty;"),
        "JsonProperty import must be present when @JsonProperty annotations are emitted"
    );
}

#[test]
fn boxed_duration_compact_ctor_only_null_checks_not_zero() {
    let typ = make_config_type_with_duration_default();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    assert!(
        out.contains("requestTimeout == null"),
        "expected null-check in compact ctor"
    );
    assert!(
        !out.contains("requestTimeout == 0"),
        "must not coerce explicit 0 — that is a user-intentional value"
    );
}

/// A type with only 2 visible fields but one carrying `#[serde(flatten)]` on a
/// `serde_json::Value` field must still emit a Builder (with `@JsonAnySetter`)
/// regardless of the Auto field-count threshold.  Without the Builder, Jackson
/// cannot absorb unknown sibling keys and throws
/// `Unrecognized field "..." not marked as ignorable`.
#[test]
fn flatten_json_field_forces_builder_emission_below_auto_threshold() {
    use crate::core::ir::CoreWrapper;
    let typ = TypeDef {
        name: "ResponseTool".to_string(),
        rust_path: "sample_llm::ResponseTool".to_string(),
        original_rust_path: "sample_llm::ResponseTool".to_string(),
        fields: vec![
            FieldDef {
                version: Default::default(),
                name: "tool_type".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: Some("\"\"".to_string()),
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: Some("type".to_string()),
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
                name: "config".to_string(),
                ty: TypeRef::Json,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: true,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let out = gen_record_type(
        "dev.sample_crate.samplellm",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleLlmRs",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );
    assert!(
        out.contains("@JsonDeserialize(builder = ResponseTool.Builder.class)"),
        "flatten+Json type must emit Builder even with < 5 fields"
    );
    assert!(
        out.contains("@com.fasterxml.jackson.annotation.JsonAnySetter"),
        "Builder must have @JsonAnySetter to absorb unknown sibling fields"
    );
    assert!(
        out.contains("@com.fasterxml.jackson.annotation.JsonAnyGetter"),
        "record field must still carry @JsonAnyGetter for serialization"
    );
}

#[test]
fn opaque_handle_close_is_idempotent_and_rejects_post_close_use() {
    let typ = TypeDef {
        name: "ResourceHandle".to_string(),
        rust_path: "sample_crate::ResourceHandle".to_string(),
        original_rust_path: "sample_crate::ResourceHandle".to_string(),
        is_opaque: true,
        ..Default::default()
    };
    let out = gen_opaque_handle_class(
        "dev.sample_crate",
        &typ,
        "sample",
        &[],
        "SampleRs",
        &AHashSet::default(),
        &AHashSet::default(),
        &AHashSet::default(),
    );

    assert!(out.contains("private MemorySegment handle;"), "{out}");
    assert!(out.contains("synchronized MemorySegment handle()"), "{out}");
    assert!(
        out.contains("throw new IllegalStateException(\"ResourceHandle is closed\")"),
        "{out}"
    );
    assert!(out.contains("public synchronized void close()"), "{out}");
    assert!(out.contains("handle = MemorySegment.NULL;"), "{out}");
    assert!(out.contains("invoke(handleToFree)"), "{out}");
}

/// The defect: `max_redirects` is a bare `usize` with a literal default and no
/// `#[serde(default)]`, so it stayed an unboxed `long` and the compact constructor
/// restored the default with `maxRedirects == 0`. A caller passing an explicit 0 —
/// "follow no redirects" — silently got 10 instead.
///
/// This is the same contract `boxed_duration_compact_ctor_only_null_checks_not_zero`
/// already states for the boxed half; that test simply never exercised the primitive
/// path. Boxing the component is what makes `== null` available as the sentinel.
#[test]
fn primitive_literal_default_never_coerces_an_explicit_zero() {
    let typ = make_config_type_with_primitive_default();
    let out = gen_record_type(
        "dev.sample_crate",
        &typ,
        &AHashSet::default(),
        &AHashSet::default(),
        "SNAKE_CASE",
        &[],
        "SampleCrawler",
        JavaBuilderMode::Auto,
        &ahash::AHashMap::default(),
        &AHashSet::default(),
        &HashSet::default(),
    );

    assert!(
        !out.contains("maxRedirects == 0"),
        "must not coerce explicit 0 — that is a user-intentional value. Emitted:\n{out}"
    );
    assert!(
        out.contains("maxRedirects == null"),
        "the default must be restored from an absent value, not from 0. Emitted:\n{out}"
    );
    assert!(
        out.contains("Long maxRedirects") || out.contains("Integer maxRedirects"),
        "the component must be boxed so null can mean \"not supplied\". Emitted:\n{out}"
    );
}
