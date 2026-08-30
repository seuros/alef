use super::*;

#[test]
fn test_struct_with_default_derive() {
    let source = r#"
        /// A configuration with sensible defaults.
        #[derive(Default, Clone)]
        pub struct Config {
            pub name: String,
            pub count: u32,
            pub enabled: bool,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let config = &surface.types[0];
    assert_eq!(config.name, "Config");
    // has_default should be true for types with #[derive(Default)]
    assert!(
        config.has_default,
        "Config with #[derive(Default)] should have has_default=true"
    );

    // The control for the whole `Empty`/`Unresolved` split. A derived `Default` gives every field
    // its type's zero, so `Empty` is an assertion here and a backend substituting its own zero is
    // exact. Widening `Unresolved` over this meaning would arm `unreadable_field_default` on
    // every `#[derive(Default)]` type in every consumer crate. ~keep
    for field in &config.fields {
        assert_eq!(
            field.typed_default,
            Some(crate::core::ir::DefaultValue::Empty),
            "`{}` is a derived type-zero and must stay `Empty`, never `Unresolved`",
            field.name
        );
    }
}

#[test]
fn test_struct_without_default() {
    let source = r#"
        /// A configuration without defaults.
        pub struct Custom {
            pub value: String,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);

    let custom = &surface.types[0];
    assert_eq!(custom.name, "Custom");
    assert!(
        !custom.has_default,
        "Struct without Default should have has_default=false"
    );
}

#[test]
fn test_serde_function_default_preserves_runtime_provider() {
    let source = r#"
        pub struct RetryPolicy {
            #[serde(default = "defaults::retry_limit")]
            pub limit: u32,
        }
    "#;

    let surface = extract_from_source(source);
    let limit = &surface.types[0].fields[0];

    assert_eq!(
        limit.typed_default,
        Some(crate::core::ir::DefaultValue::FunctionCall(
            "defaults::retry_limit".to_string()
        ))
    );
}

#[test]
fn public_associated_serde_default_is_resolved_as_callable() {
    let source = r#"
        pub struct NetworkPolicy;

        impl NetworkPolicy {
            pub fn from_environment() -> Self {
                Self
            }
        }

        pub struct ClientConfig {
            #[serde(default = "NetworkPolicy::from_environment")]
            pub policy: NetworkPolicy,
            pub nested: RequiredSettings,
        }

        pub struct RequiredSettings {
            pub label: String,
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "ClientConfig").unwrap();
    let policy = config.fields.iter().find(|field| field.name == "policy").unwrap();

    assert_eq!(
        policy.typed_default,
        Some(crate::core::ir::DefaultValue::PublicFunctionCall(
            "test_crate::NetworkPolicy::from_environment".to_string()
        ))
    );
}

#[test]
fn test_impl_default_without_fn_default() {
    let source = r#"
        pub struct Incomplete {
            pub value: u32,
        }

        impl Default for Incomplete {
            // Missing fn default() - no matching method
        }
    "#;

    let surface = extract_from_source(source);
    let incomplete = &surface.types[0];
    let value_field = &incomplete.fields[0];

    // Contract reversed deliberately. This used to record `Empty`, which every backend renders as
    // the target-language zero. But an `impl Default` whose body we cannot read is not a claim that
    // the default IS the zero -- it is the absence of a reading. Conflating the two is what shipped
    // `DetDbThresh = 0.0f` beneath a generated doc comment reading "default: 0.3". ~keep
    assert_eq!(
        value_field.typed_default,
        Some(crate::core::ir::DefaultValue::Unresolved(
            "impl Default block without a `fn default()` item".to_string()
        )),
        "an unreadable impl Default must be Unresolved, not Empty"
    );
    assert_ne!(
        value_field.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "Empty licenses every backend to substitute its own zero for a default it never read"
    );
}

#[test]
fn test_manual_default_impl_exposes_static_constructor() {
    let source = r#"
        pub struct Settings {
            pub enabled: bool,
        }

        impl Default for Settings {
            fn default() -> Self {
                Self { enabled: true }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let settings = surface.types.iter().find(|typ| typ.name == "Settings").unwrap();
    let default = settings.methods.iter().find(|method| method.name == "default").unwrap();

    assert!(default.is_static);
    assert!(default.params.is_empty());
    assert!(matches!(&default.return_type, crate::core::ir::TypeRef::Named(name) if name == "Settings"));
}

#[test]
fn test_enum_with_default_derive_and_default_variant() {
    let source = r#"
        #[derive(Default, Clone)]
        pub enum Priority {
            #[default]
            Normal,
            High,
            Low,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);

    let priority = &surface.enums[0];
    assert_eq!(priority.name, "Priority");
    assert_eq!(priority.variants.len(), 3);

    let normal = &priority.variants[0];
    assert_eq!(normal.name, "Normal");
    assert!(
        normal.is_default,
        "Normal variant with #[default] should have is_default=true"
    );

    let high = &priority.variants[1];
    assert_eq!(high.name, "High");
    assert!(!high.is_default, "Non-default variant should have is_default=false");

    let low = &priority.variants[2];
    assert_eq!(low.name, "Low");
    assert!(!low.is_default);
}

#[test]
fn test_enum_without_default() {
    let source = r#"
        pub enum Format {
            Json,
            Xml,
            Yaml,
        }
    "#;

    let surface = extract_from_source(source);
    let format = &surface.enums[0];

    for variant in &format.variants {
        assert!(
            !variant.is_default,
            "Variants without #[default] should be is_default=false"
        );
    }
}

#[test]
fn test_enum_with_manual_default_impl() {
    let source = r#"
        pub enum ClassificationMode {
            Known,
            Custom(String),
        }

        impl Default for ClassificationMode {
            fn default() -> Self {
                Self::Custom(String::new())
            }
        }
    "#;

    let surface = extract_from_source(source);
    let mode = &surface.enums[0];

    assert!(mode.has_default, "manual Default impl should set has_default=true");
    assert!(
        mode.variants.iter().all(|variant| !variant.is_default),
        "manual enum Default impls should not synthesize a default variant"
    );
}

#[test]
fn container_level_serde_default_is_recorded_on_the_type() {
    let source = r#"
        #[derive(serde::Serialize, serde::Deserialize)]
        #[serde(default)]
        pub struct RetryPolicy {
            pub limit: u32,
        }

        impl Default for RetryPolicy {
            fn default() -> Self {
                Self { limit: 3 }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let policy = &surface.types[0];

    assert!(
        policy.serde_container_default,
        "a container-level #[serde(default)] must set serde_container_default=true"
    );
    assert_eq!(
        policy.fields[0].default, None,
        "the container attribute must not be mistaken for a per-field #[serde(default)]"
    );
    // The non-zero value is what the container default actually yields for a missing key,
    // and what backends compare against the target language's zero value. ~keep
    assert_eq!(
        policy.fields[0].typed_default,
        Some(crate::core::ir::DefaultValue::IntLiteral(3))
    );
}

#[test]
fn deriving_default_alone_does_not_set_the_container_serde_flag() {
    let source = r#"
        /// Every key stays required on the wire even though the type has a Default impl.
        #[derive(Default, serde::Serialize, serde::Deserialize)]
        pub struct RetryPolicy {
            pub limit: u32,
        }
    "#;

    let surface = extract_from_source(source);
    let policy = &surface.types[0];

    assert!(policy.has_default, "derive(Default) still sets has_default");
    assert!(
        !policy.serde_container_default,
        "derive(Default) is not #[serde(default)]: every field is still a required wire key"
    );
}

/// Proves the exact fixture shape `codegen::config_gen`'s JSON-probe predicate depends on,
/// through real extraction rather than a hand-built `FieldDef` literal: on a `#[derive(Default)]`
/// type, `#[derive(Default)]`'s blanket seeding (`extract::extractor::types::extract_struct`)
/// unconditionally overwrites `typed_default` to `Empty` on every field — including one that
/// already carries a genuine `#[serde(default = "path")]` — so `typed_default.is_some()` is true
/// for a field serde will fill on a missing key (`count`) and one that is fully required
/// (`label`) alike. Only `FieldDef::default`, set once by `extract_field` from the real
/// attribute and never touched by the later derive/impl overwrite, tells the two apart. This is
/// the production counterpart to the hand-built fixtures in
/// `codegen::config_gen::tests::derive_default_probe` and the (now-repaired) `grid_cell_type` in
/// `codegen::config_gen::tests::defaults` — it fails if extraction ever stops producing this
/// shape, which would make those fixtures fictions again. ~keep
#[test]
fn derive_default_seeds_empty_over_a_genuine_field_level_serde_default() {
    let source = r#"
        #[derive(Default, serde::Serialize, serde::Deserialize)]
        pub struct DerivedConfig {
            #[serde(default = "mylib::default_count")]
            pub count: u32,
            pub label: String,
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "DerivedConfig").unwrap();
    let count = config.fields.iter().find(|field| field.name == "count").unwrap();
    let label = config.fields.iter().find(|field| field.name == "label").unwrap();

    assert_eq!(
        count.default.as_deref(),
        Some("serde(default = \"mylib::default_count\")"),
        "the genuine field-level serde default must survive the derive(Default) overwrite"
    );
    assert_eq!(
        label.default, None,
        "a field with no serde attribute of its own must not gain a default"
    );
    assert_eq!(
        count.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "derive(Default) overwrites even a serde-defaulted field's typed_default to Empty"
    );
    assert_eq!(
        label.typed_default,
        Some(crate::core::ir::DefaultValue::Empty),
        "typed_default alone cannot distinguish `count` (serde will fill it) from `label` \
         (fully required): both read Empty"
    );
}
