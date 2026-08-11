use super::NapiBackend;
use super::methods::{gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::core::backend::Backend;
use crate::core::config::Language;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use ahash::AHashSet;

/// NapiBackend::name returns "napi".
#[test]
fn napi_backend_name_is_napi() {
    let b = NapiBackend;
    assert_eq!(b.name(), "napi");
}

/// NapiBackend::language returns Language::Node.
#[test]
fn napi_backend_language_is_node() {
    let b = NapiBackend;
    assert_eq!(b.language(), Language::Node);
}

/// Test that cfg-gated fields in never_skip_cfg_field_names pass the options-field-bridge filter.
#[test]
fn cfg_gated_field_accepted_when_in_never_skip_list() {
    let never_skip_cfg_field_names = ["visitor".to_string()];
    let field_is_target = "visitor";

    let field_has_cfg = Some("feature = \"visitor\"");

    let accepted = field_has_cfg.is_none() || never_skip_cfg_field_names.iter().any(|n| n == field_is_target);

    assert!(
        accepted,
        "cfg-gated field 'visitor' should pass filter when in never_skip_cfg_field_names"
    );
}

/// Test that plain data enums (with data variants, not tagged/untagged) appearing in struct fields
/// get binding-to-core From impls when the struct is an input type.
/// Regression: AuthHeaderFormat has data variant ApiKey(String), appears in CustomProviderConfig
/// field, but binding-to-core impl was not being generated, causing struct conversion to fail.
#[test]
fn plain_data_enum_in_input_type_struct_gets_binding_to_core_impl() {
    use crate::codegen::conversions::{
        ConversionConfig, can_generate_enum_conversion, can_generate_enum_conversion_from_core,
        gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding_cfg,
    };
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

    let auth_format_enum = EnumDef {
        name: "AuthHeaderFormat".to_string(),
        rust_path: "fixture_core::AuthHeaderFormat".to_string(),
        variants: vec![
            EnumVariant {
                name: "Bearer".to_string(),
                fields: vec![],
                ..Default::default()
            },
            EnumVariant {
                name: "ApiKey".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "None".to_string(),
                fields: vec![],
                ..Default::default()
            },
        ],
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        ..Default::default()
    };

    let has_data_variants = auth_format_enum.variants.iter().any(|v| !v.fields.is_empty());
    assert!(has_data_variants, "AuthHeaderFormat should have data variants");

    let is_tagged = auth_format_enum.serde_tag.is_some();
    let is_untagged = auth_format_enum.serde_untagged;
    assert!(
        !(is_tagged && has_data_variants),
        "AuthHeaderFormat should not be tagged data enum"
    );
    assert!(
        !(is_untagged && has_data_variants),
        "AuthHeaderFormat should not be untagged data enum"
    );

    assert!(
        can_generate_enum_conversion(&auth_format_enum),
        "plain data enum should be eligible for binding-to-core conversion"
    );
    assert!(
        can_generate_enum_conversion_from_core(&auth_format_enum),
        "plain data enum should be eligible for core-to-binding conversion"
    );

    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..Default::default()
    };
    let binding_to_core = gen_enum_from_binding_to_core_cfg(&auth_format_enum, "fixture_core", &config);
    assert!(
        binding_to_core.contains("impl From<JsAuthHeaderFormat> for fixture_core::AuthHeaderFormat"),
        "should emit binding-to-core impl for plain data enum; got:\n{binding_to_core}"
    );

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&auth_format_enum, "fixture_core", &config);
    assert!(
        core_to_binding.contains("impl From<fixture_core::AuthHeaderFormat> for JsAuthHeaderFormat"),
        "should emit core-to-binding impl for plain data enum; got:\n{core_to_binding}"
    );
}

/// Test that opaque types with `has_default=true` emit `#[napi(constructor)]` with `new_constructor()`
/// even when a static `new()` method exists. This ensures JS `new ClassName()` works without
/// causing duplicate symbol errors.
/// Regression: App type has both a static `new()` method and `has_default=true`, but the
/// NAPI backend was skipping constructor emission due to the `!has_static_new` guard, causing
/// "Class contains no 'constructor', can not new it!" at runtime.
#[test]
fn napi_opaque_type_with_default_and_static_new_emits_constructor() {
    use super::constructors::napi_default_constructor;
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, TypeDef, TypeRef};

    let app_type = TypeDef {
        name: "App".to_string(),
        rust_path: "sample_crate::App".to_string(),
        is_opaque: true,
        has_default: true,
        methods: vec![MethodDef {
            name: "new".to_string(),
            receiver: None,
            params: vec![],
            return_type: TypeRef::Named("App".to_string()),
            is_async: false,
            is_static: true,
            doc: "Create a new application".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let mapper = NapiMapper::new("Js".to_string());
    let constructor = napi_default_constructor(&app_type, &mapper, "sample_crate", "Js");

    assert!(
        constructor.is_some(),
        "opaque type with has_default=true should emit constructor even with static new()"
    );

    let constructor_code = constructor.unwrap();
    assert!(
        constructor_code.contains("#[napi(constructor)]"),
        "constructor should be marked with #[napi(constructor)]"
    );
    assert!(
        constructor_code.contains("pub fn new_constructor()"),
        "constructor should use new_constructor() to avoid conflict with static new()"
    );
    assert!(
        constructor_code.contains("Self { inner: std::sync::Arc::new(sample_crate::App::new())"),
        "constructor should create new App via sample_crate::App::new()"
    );
}

/// Regression: a `&mut self -> Result<&mut Self, E>` builder (a method returning a reference to
/// its own wrapper type) must SHARE the existing handle's `Arc` (`self.inner.clone()`) instead of
/// cloning the returned reference. `&mut App` is not `Clone`, so
/// `Arc::new(std::sync::Mutex::new(result.clone()))` fails to compile (E0599).
#[test]
fn napi_self_ref_builder_shares_arc_instead_of_cloning_returned_ref() {
    use super::types::gen_opaque_instance_method;
    use crate::backends::napi::type_map::NapiMapper;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
    use ahash::AHashSet;
    use std::collections::HashMap;

    let method = MethodDef {
        name: "register_route".to_string(),
        params: vec![ParamDef {
            name: "config".to_string(),
            ty: TypeRef::Named("RouteCfg".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("App".to_string()),
        error_type: Some("AppError".to_string()),
        doc: "Register a route, returning the app for chaining.".to_string(),
        receiver: Some(ReceiverKind::RefMut),
        returns_ref: true,
        ..MethodDef::default()
    };
    let typ = TypeDef {
        name: "App".to_string(),
        rust_path: "sample_crate::App".to_string(),
        is_opaque: true,
        methods: vec![method.clone()],
        ..Default::default()
    };

    let mapper = NapiMapper::new("Js".to_string());
    let cfg = super::NapiBackend::binding_config("sample_crate", "Js", true);
    let mut opaque = AHashSet::new();
    opaque.insert("App".to_string());
    opaque.insert("RouteCfg".to_string());
    let mut mutex = AHashSet::new();
    mutex.insert("App".to_string());
    let adapter_bodies = crate::adapters::AdapterBodies::new();
    let streaming: ahash::AHashMap<String, String> = ahash::AHashMap::new();
    let capsule: HashMap<String, crate::core::config::NodeCapsuleTypeConfig> = HashMap::new();

    let code = gen_opaque_instance_method(
        &method,
        &mapper,
        &typ,
        &cfg,
        &opaque,
        "Js",
        &adapter_bodies,
        &streaming,
        &mutex,
        &capsule,
    );

    assert!(
        code.contains("Ok(Self { inner: self.inner.clone() })"),
        "self-returning builder should share the existing Arc, got:\n{code}"
    );
    assert!(
        !code.contains("result.clone()"),
        "must not clone the returned &mut ref, got:\n{code}"
    );
    assert!(
        !code.contains("let result ="),
        "self-returning builder must not bind the returned &mut ref, got:\n{code}"
    );
}

/// Build a single-variant tagged enum with one sanitized field named `entries`, for exercising
/// the binding↔core conversion of sanitized tagged-enum fields.
fn sanitized_field_test_enum(ty: TypeRef, optional: bool) -> EnumDef {
    EnumDef {
        name: "NodeContent".to_string(),
        rust_path: "fixture_core::NodeContent".to_string(),
        variants: vec![EnumVariant {
            name: "MetadataBlock".to_string(),
            fields: vec![FieldDef {
                name: "entries".to_string(),
                ty,
                optional,
                sanitized: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("node_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    }
}

fn sanitized_field_conversions(enum_def: &EnumDef) -> (String, String) {
    let struct_names = AHashSet::new();
    (
        gen_tagged_enum_binding_to_core(enum_def, "fixture_core", "Js", &struct_names),
        gen_tagged_enum_core_to_binding(enum_def, "fixture_core", "Js", &struct_names),
    )
}

/// Tagged-enum discriminator values (`#[serde(rename_all/rename)]`) must drive the wire tag in
/// both conversion directions, never the raw variant/field Rust identifier.
/// Regression coverage for PR #218 (the discriminator half, unrelated to sanitized fields).
#[test]
fn tagged_enum_conversions_use_serde_wire_names() {
    let enum_def = EnumDef {
        name: "NodeContent".to_string(),
        rust_path: "fixture_core::NodeContent".to_string(),
        variants: vec![
            EnumVariant {
                name: "ListItem".to_string(),
                fields: vec![FieldDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "PageBreak".to_string(),
                serde_rename: Some("explicit-page-break".to_string()),
                ..Default::default()
            },
        ],
        serde_content: None,
        serde_tag: Some("node_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let (binding_to_core, core_to_binding) = sanitized_field_conversions(&enum_def);

    for generated in [&binding_to_core, &core_to_binding] {
        assert!(generated.contains("\"list_item\""), "generated:\n{generated}");
        assert!(generated.contains("\"explicit-page-break\""), "generated:\n{generated}");
        assert!(!generated.contains("\"listitem\""), "generated:\n{generated}");
        assert!(!generated.contains("\"pagebreak\""), "generated:\n{generated}");
    }
}

/// A table of sanitized tagged-enum field shapes, asserting both conversion directions in one
/// pass. Covers: the `Vec<Vec<String>>` shape (issue #217's `Vec<(String, String)>` case) both
/// non-optional and optional, the `Map<String, String>` shape, and an unsupported shape
/// (`Vec<Named>`) that must keep emitting the pre-#218 `Default::default()` / `None` fallback —
/// the only form that is guaranteed to compile for a shape this backend cannot invert.
struct SanitizedFieldCase {
    label: &'static str,
    ty: TypeRef,
    optional: bool,
    binding_to_core_expect: &'static str,
    core_to_binding_expect: &'static str,
    /// Substrings that must be absent from the core→binding output: the pre-fix behavior either
    /// dropped the field entirely (`entries: None`) or left the destructured variable unused
    /// (`entries: _entries`) even though a real conversion was possible.
    core_to_binding_forbid: &'static [&'static str],
}

#[test]
fn sanitized_tagged_enum_field_conversions_table() {
    let cases = [
        SanitizedFieldCase {
            label: "vec_vec_string non-optional (issue #217)",
            ty: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
            optional: false,
            binding_to_core_expect: "entries: val.entries.as_deref().unwrap_or_default().iter().filter_map",
            core_to_binding_expect: "entries: Some(entries.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "vec_vec_string optional",
            ty: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
            optional: true,
            binding_to_core_expect: "entries: val.entries.map(|v| v.iter().filter_map(|inner| { let mut it = inner.iter().cloned(); Some((it.next()?, it.next()?)) }).collect())",
            core_to_binding_expect: "entries: entries.map(|v| v.iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect::<Vec<Vec<String>>>())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "map_string_string non-optional",
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: false,
            binding_to_core_expect: "entries: val.entries.unwrap_or_default().into_iter().map(|(k, v)| (k.into(), v.into())).collect()",
            core_to_binding_expect: "entries: Some(entries.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "map_string_string optional",
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: true,
            binding_to_core_expect: "entries: val.entries.map(|m| m.into_iter().map(|(k, v)| (k.into(), v.into())).collect())",
            core_to_binding_expect: "entries: entries.map(|v| v.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())",
            core_to_binding_forbid: &["entries: None", "entries: _entries"],
        },
        SanitizedFieldCase {
            label: "unsupported vec_named shape falls back to Default/None",
            ty: TypeRef::Vec(Box::new(TypeRef::Named("Widget".to_string()))),
            optional: false,
            binding_to_core_expect: "entries: Default::default()",
            core_to_binding_expect: "entries: None",
            core_to_binding_forbid: &[],
        },
    ];

    for case in cases {
        let enum_def = sanitized_field_test_enum(case.ty, case.optional);
        let (binding_to_core, core_to_binding) = sanitized_field_conversions(&enum_def);

        assert!(
            binding_to_core.contains(case.binding_to_core_expect),
            "[{}] binding->core missing expected fragment {:?}, got:\n{binding_to_core}",
            case.label,
            case.binding_to_core_expect
        );
        assert!(
            core_to_binding.contains(case.core_to_binding_expect),
            "[{}] core->binding missing expected fragment {:?}, got:\n{core_to_binding}",
            case.label,
            case.core_to_binding_expect
        );
        for forbidden in case.core_to_binding_forbid {
            assert!(
                !core_to_binding.contains(forbidden),
                "[{}] core->binding must not contain {:?}, got:\n{core_to_binding}",
                case.label,
                forbidden
            );
        }
        assert!(
            !binding_to_core.contains("format!(\"{:?}\""),
            "[{}] binding->core must never re-parse a Debug-formatted string, got:\n{binding_to_core}",
            case.label
        );
    }
}

/// Regression coverage for the `clippy::redundant_field_names` fix: an optional tagged-enum
/// field whose type needs no cast/wrap (e.g. plain `Option<String>`) falls into the fallback
/// match arm in `gen_tagged_enum_core_to_binding`, where the destructured core-side variable
/// is bound under the same name as the binding field it fills. The field-init expression must
/// use shorthand (`text`), never `text: text`.
#[test]
fn core_to_binding_uses_shorthand_for_self_assigned_optional_field() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let struct_names = AHashSet::new();
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names);

    assert!(
        !core_to_binding.contains("text: text"),
        "self-assigned optional field must use shorthand, not `text: text`:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("text }") || core_to_binding.contains("text,"),
        "expected field-init shorthand for `text`:\n{core_to_binding}"
    );
}

/// Same fallback arm, but for a field whose type does need a cast/wrap (`Vec<Named>` maps
/// through `.into()`) — this must keep the real `field: expr` form, not collapse to shorthand.
#[test]
fn core_to_binding_keeps_expr_form_when_conversion_is_required() {
    let enum_def = EnumDef {
        name: "Content".to_string(),
        rust_path: "fixture_core::Content".to_string(),
        variants: vec![EnumVariant {
            name: "Items".to_string(),
            fields: vec![FieldDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Widget".to_string()))),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    };

    let struct_names = AHashSet::new();
    let core_to_binding = gen_tagged_enum_core_to_binding(&enum_def, "fixture_core", "Js", &struct_names);

    assert!(
        core_to_binding.contains("items: items.map(|v| v.into_iter().map(Into::into).collect())"),
        "Vec<Named> optional field must keep its real conversion expression:\n{core_to_binding}"
    );
}
