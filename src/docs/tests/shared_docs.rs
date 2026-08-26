use super::super::shared_pages::render_enum_for_shared_doc;
use super::*;

/// A minimal `types.md`-triggering config type: any type whose name ends in `Config` lands
/// in the "Configuration Types" category, which is what emits the cross-page link to
/// `configuration.md` under test here.
fn make_config_type() -> TypeDef {
    TypeDef {
        name: "FooConfig".into(),
        rust_path: "mylib::FooConfig".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// task: `types.md`'s link to `configuration.md` was hardcoded to a `.md`-suffixed relative
/// link, which a content-collection docs site (Astro Starlight) cannot resolve -- it needs
/// `./configuration/`. The default (no `[docs].reference_link_style` set) must keep emitting
/// the `.md`-suffixed form so plain Markdown viewers and unconfigured consumers see no
/// change; a consumer opting into `reference_link_style = "extensionless"` must get the
/// directory-style route instead, and only that.
#[test]
fn test_types_doc_configuration_link_honors_reference_link_style() {
    let api = ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        types: vec![make_config_type()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let default_config = make_test_config();
    let default_files = generate_docs(&api, &default_config, &[Language::Python], "out").unwrap();
    let default_types_file = default_files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("types"))
        .unwrap();
    assert!(
        default_types_file
            .content
            .contains("[Configuration Reference](configuration.md)"),
        "default (unconfigured) link style must stay `.md`-suffixed for backward compatibility: {}",
        default_types_file.content
    );

    let extensionless_raw: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.docs]
reference_link_style = "extensionless"

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    )
    .expect("valid toml");
    let extensionless_config = extensionless_raw.resolve().expect("resolve ok").remove(0);
    let extensionless_files = generate_docs(&api, &extensionless_config, &[Language::Python], "out").unwrap();
    let extensionless_types_file = extensionless_files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("types"))
        .unwrap();
    assert!(
        extensionless_types_file
            .content
            .contains("[Configuration Reference](./configuration/)"),
        "`reference_link_style = \"extensionless\"` must emit a directory-style route, not a \
         `.md`-suffixed link a Starlight-style site cannot resolve: {}",
        extensionless_types_file.content
    );
    assert!(
        !extensionless_types_file.content.contains("configuration.md"),
        "the `.md`-suffixed form must not leak through once extensionless is configured: {}",
        extensionless_types_file.content
    );
}

#[test]
fn test_generate_types_doc_renders_enum_variants() {
    use crate::core::ir::EnumVariant;
    let api = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "TableModel".into(),
            rust_path: "test::TableModel".into(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Tatr".into(),
                    fields: vec![],
                    doc: "TATR transformer (default).".into(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "SlanetWired".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Table structure model.".into(),
            cfg: None,
            is_copy: true,
            has_serde: true,
            has_default: false,
            serde_content: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let types_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("types"))
        .unwrap();
    assert!(types_file.content.contains("### Enums"));
    assert!(types_file.content.contains("#### TableModel"));
    assert!(types_file.content.contains("Table structure model."));
    assert!(types_file.content.contains("`Tatr`"));
    assert!(types_file.content.contains("TATR transformer"));
    assert!(types_file.content.contains("`SlanetWired`"));
}

#[test]
fn test_render_enum_for_shared_doc_emits_wire_value_column_when_rename_all_set() {
    use crate::core::ir::EnumVariant;
    let en = EnumDef {
        name: "HtmlTheme".into(),
        rust_path: "test::HtmlTheme".into(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Default".into(),
                fields: vec![],
                doc: "Default theme.".into(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Github".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: "HTML theme.".into(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: Some("lowercase".into()),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let out = render_enum_for_shared_doc(&en, Language::Rust);
    assert!(out.contains("| Variant | Wire value | Description |"));
    assert!(out.contains("| `Default` | `default` |"));
    assert!(out.contains("| `Github` | `github` |"));
}

#[test]
fn test_render_enum_for_shared_doc_demotes_internal_headings() {
    use crate::core::ir::EnumVariant;
    let en = EnumDef {
        name: "OutputFormat".into(),
        rust_path: "test::OutputFormat".into(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Markdown".into(),
            fields: vec![],
            doc: String::new(),
            is_default: true,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: "Output format specification.\n\n## Variants\n\nDetailed variant info.".into(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let out = render_enum_for_shared_doc(&en, Language::Rust);
    assert!(
        out.contains("#### Variants"),
        "internal heading must be demoted to #### (was ##): {out}"
    );
    assert!(
        !out.lines().any(|l| l == "## Variants"),
        "raw ## heading must not remain: {out}"
    );
    assert!(out.contains("Output format specification."));
}

#[test]
fn test_generate_configuration_doc_renders_referenced_enums_only() {
    use crate::core::ir::{CoreWrapper, EnumVariant, FieldDef};
    let api = ApiSurface {
        crate_name: "mylib".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "ImageConfig".into(),
            rust_path: "mylib::ImageConfig".into(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                version: Default::default(),
                name: "format".into(),
                ty: TypeRef::Named("mylib::ImageFormat".into()),
                optional: false,
                default: None,
                doc: "Output image format.".into(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "Image config.".into(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![
            EnumDef {
                name: "ImageFormat".into(),
                rust_path: "mylib::ImageFormat".into(),
                original_rust_path: String::new(),
                variants: vec![EnumVariant {
                    name: "Png".into(),
                    fields: vec![],
                    doc: "PNG output.".into(),
                    is_default: true,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                }],
                methods: vec![],
                doc: "Image format enum backed by `tl::parse`.".into(),
                cfg: None,
                is_copy: true,
                has_serde: true,
                serde_content: None,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
                has_default: false,
            },
            EnumDef {
                name: "Unrelated".into(),
                rust_path: "mylib::Unrelated".into(),
                original_rust_path: String::new(),
                variants: vec![EnumVariant {
                    name: "A".into(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                }],
                methods: vec![],
                doc: "Not referenced by any config type.".into(),
                cfg: None,
                is_copy: true,
                has_serde: true,
                serde_content: None,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
                version: Default::default(),
                has_default: false,
            },
        ],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    let cfg_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("configuration"))
        .unwrap();
    assert!(cfg_file.content.contains("### Enums"));
    assert!(cfg_file.content.contains("#### ImageFormat"));
    assert!(cfg_file.content.contains("`tl.parse`"));
    assert!(!cfg_file.content.contains("`tl::parse`"));
    assert!(
        !cfg_file.content.contains("#### Unrelated"),
        "configuration.md must filter out enums not referenced by any config-type field"
    );
}
