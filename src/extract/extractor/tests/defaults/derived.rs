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

/// The root of the collapsed-collection-default family. `#[derive(Default)]` used to blanket-write
/// `DefaultValue::Empty` over every field, destroying the `FunctionCall` that
/// `helpers::fields::extract_field` records for `#[serde(default = "path")]`. Only the marker
/// string in `FieldDef::default` survived, so every backend keying its refusal on `FunctionCall`
/// (Kotlin, Swift, C#) saw `Empty` — "the type's zero, exactly" — and fabricated an empty
/// collection over a populated one.
///
/// The two attributes are not redundant and do not agree: an absent wire key is filled by `path()`
/// under `#[serde(default = "path")]`, never by `Default::default()`. `Empty` must therefore lose
/// to a recorded `FunctionCall`, on collections and scalars alike. ~keep
#[test]
fn derive_default_does_not_erase_a_named_serde_default() {
    use crate::core::ir::DefaultValue;

    let source = r#"
        #[derive(Default, serde::Serialize, serde::Deserialize)]
        pub struct SecurityPolicy {
            #[serde(default = "default_scheme_allowlist")]
            pub scheme_allowlist: Vec<String>,
            #[serde(default = "default_header_overrides")]
            pub header_overrides: std::collections::HashMap<String, String>,
            #[serde(default = "default_redirect_limit")]
            pub redirect_limit: u32,
        }
    "#;

    let surface = extract_from_source(source);
    let policy = &surface.types[0];
    let field = |name: &str| {
        policy
            .fields
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("`{name}` is missing from the extracted surface"))
    };

    assert!(policy.has_default, "the fixture must still derive Default");
    assert_eq!(
        field("scheme_allowlist").typed_default,
        Some(DefaultValue::FunctionCall("default_scheme_allowlist".to_string())),
        "a Vec's named serde default must survive derive(Default); `Empty` here licenses every \
         backend to ship an empty allow-list in place of the real one"
    );
    assert_eq!(
        field("header_overrides").typed_default,
        Some(DefaultValue::FunctionCall("default_header_overrides".to_string())),
        "a Map's named serde default must survive derive(Default)"
    );
    assert_eq!(
        field("redirect_limit").typed_default,
        Some(DefaultValue::FunctionCall("default_redirect_limit".to_string())),
        "a scalar's named serde default must survive derive(Default)"
    );
}

/// Discrimination control for the test above. Precedence must not become suppression: the seeding
/// of `Empty` is correct for every field that has no named default of its own, and a fix that
/// simply stopped writing `Empty` under `#[derive(Default)]` would satisfy the assertions above
/// while stripping the type-zero assertion off every ordinary field in every consumer crate.
///
/// A *bare* `#[serde(default)]` is the sharpest case: it records no `typed_default` at all, so it
/// must still land on `Empty` — for it, `Default::default()` genuinely is the value, and on a
/// `Vec` that is exactly the empty list. ~keep
#[test]
fn derive_default_still_seeds_empty_where_no_named_default_was_recorded() {
    use crate::core::ir::DefaultValue;

    let source = r#"
        #[derive(Default, serde::Serialize, serde::Deserialize)]
        pub struct SecurityPolicy {
            #[serde(default)]
            pub scheme_allowlist: Vec<String>,
            #[serde(default)]
            pub header_overrides: std::collections::HashMap<String, String>,
            pub redirect_limit: u32,
            pub user_agent: String,
        }
    "#;

    let surface = extract_from_source(source);
    let policy = &surface.types[0];

    for field in &policy.fields {
        assert_eq!(
            field.typed_default,
            Some(DefaultValue::Empty),
            "`{}` carries no named serde default, so the derived type-zero assertion still applies",
            field.name
        );
    }
}
