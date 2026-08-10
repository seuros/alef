use super::*;

#[test]
fn test_has_serde_via_derive_still_detected() {
    // Regression: types using #[derive(Serialize, Deserialize)] must still get has_serde=true.
    let source = r#"
        #[derive(Clone, serde::Serialize, serde::Deserialize)]
        pub struct Config {
            pub name: String,
            pub timeout: u64,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(surface.types[0].has_serde, "derive-based serde must still be detected");
}

#[test]
fn test_has_serde_via_manual_impls_detected() {
    let source = r#"
        #[derive(Clone, Debug)]
        pub struct NodeContext {
            pub tag_name: String,
            pub depth: usize,
        }

        impl serde::Serialize for NodeContext {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                unimplemented!()
            }
        }

        impl<'de> serde::Deserialize<'de> for NodeContext {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(
        surface.types[0].has_serde,
        "manual impl Serialize + impl Deserialize must set has_serde=true"
    );
}

#[test]
fn test_has_serde_with_lifetime_parameterised_manual_impls() {
    let source = r#"
        #[derive(Clone, Debug)]
        pub struct Foo {
            pub value: String,
        }

        impl serde::Serialize for Foo {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                unimplemented!()
            }
        }

        impl<'de> serde::Deserialize<'de> for Foo {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(
        surface.types[0].has_serde,
        "lifetime-parameterised manual serde impls must set has_serde=true"
    );
}

#[test]
fn test_has_serde_only_serialize_not_set() {
    let source = r#"
        #[derive(Clone, Debug)]
        pub struct Foo {
            pub value: String,
        }

        impl serde::Serialize for Foo {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                unimplemented!()
            }
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(
        !surface.types[0].has_serde,
        "only Serialize without Deserialize must not set has_serde"
    );
}

#[test]
fn test_internally_tagged_enum_preserves_named_value_fields() {
    let source = r#"
        #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        pub enum MatchRule {
            Exact { value: String },
            Suffix { value: String },
            Pattern { value: String },
        }
    "#;

    let surface = extract_from_source(source);
    let enum_def = surface.enums.first().expect("MatchRule should be extracted");

    assert_eq!(enum_def.name, "MatchRule");
    assert_eq!(enum_def.serde_tag.as_deref(), Some("kind"));
    assert_eq!(enum_def.serde_rename_all.as_deref(), Some("snake_case"));
    assert_eq!(enum_def.variants.len(), 3);
    for variant in &enum_def.variants {
        assert_eq!(variant.fields.len(), 1, "{} must retain its payload", variant.name);
        assert_eq!(variant.fields[0].name, "value");
        assert_eq!(variant.fields[0].ty, TypeRef::String);
        assert!(!variant.is_tuple);
    }
}

#[test]
fn test_enum_rename_all_under_cfg_attr_any_is_honoured() {
    // Regression for the html-to-markdown `TierStrategy` bug: `rename_all` living inside
    // `#[cfg_attr(any(...), serde(rename_all = "..."))]` used to be invisible to Alef's
    // extractor, so the enum's `serde_rename_all` came back `None` and the Java backend fell
    // back to emitting the bare PascalCase variant name (`Auto`) instead of the wire name the
    // Rust core actually deserializes (`auto`).
    let source = r#"
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        #[cfg_attr(any(feature = "serde", feature = "metadata"), derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(any(feature = "serde", feature = "metadata"), serde(rename_all = "snake_case"))]
        pub enum TierStrategy {
            #[default]
            Auto,
            Tier2,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 1);
    let enum_def = &surface.enums[0];
    assert!(
        enum_def.has_serde,
        "cfg_attr-gated derive(Serialize, Deserialize) must set has_serde"
    );
    assert_eq!(
        enum_def.serde_rename_all.as_deref(),
        Some("snake_case"),
        "rename_all nested inside cfg_attr(any(...), ...) must be extracted"
    );

    let auto_variant = enum_def
        .variants
        .iter()
        .find(|v| v.name == "Auto")
        .expect("Auto variant present");
    let wire_name = crate::backends::java::gen_bindings::helpers::java_apply_rename_all(
        &auto_variant.name,
        enum_def.serde_rename_all.as_deref(),
    );
    assert_eq!(
        wire_name, "auto",
        "Java wire name must honour the cfg_attr-gated rename_all"
    );
}

#[test]
fn test_extract_function_since_annotation_is_populated() {
    let source = r#"
        #[alef(since = "1.0.0")]
        pub fn new_api() {}
    "#;
    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 1);
    assert_eq!(surface.functions[0].version.since.as_deref(), Some("1.0.0"));
    assert!(surface.functions[0].version.deprecated.is_none());
}
