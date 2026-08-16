use crate::codegen::naming::to_class_name;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{BridgeBinding, JavaBuilderMode, Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;
use std::path::PathBuf;

mod facade;
mod ffi_class;
pub mod helpers;
mod line_wrap;
mod marshal;
mod native_lib;
mod service_api;
pub mod trait_bridge;
mod types;

use facade::gen_facade_class;
use ffi_class::gen_main_class;
use helpers::{gen_exception_class, gen_infrastructure_exception_class, gen_json_util_class};
use native_lib::gen_native_lib;
use types::{gen_byte_array_serializer, gen_enum_class, gen_opaque_handle_class, gen_record_type};

pub struct JavaBackend;

impl JavaBackend {
    /// Convert crate name to main class name (PascalCase + "Rs" suffix).
    ///
    /// The "Rs" suffix ensures the raw FFI wrapper class has a distinct name from
    /// the public facade class (which strips the "Rs" suffix). Without this, the
    /// facade would delegate to itself, causing infinite recursion.
    fn resolve_main_class(api: &ApiSurface) -> String {
        let base = to_class_name(&api.crate_name.replace('-', "_"));
        if base.ends_with("Rs") {
            base
        } else {
            format!("{}Rs", base)
        }
    }
}

fn effective_exclude_types(api: &ApiSurface, config: &ResolvedCrateConfig) -> HashSet<String> {
    let mut exclude_types: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(java) = &config.java {
        exclude_types.extend(java.exclude_types.iter().cloned());
    }
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(
        api.types
            .iter()
            .filter(|t| t.has_lifetime_params)
            .map(|t| t.name.clone()),
    );
    exclude_types.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
    exclude_types
}

fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

fn signature_references_excluded_type(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

fn service_references_excluded_type(service: &crate::core::ir::ServiceDef, excluded: &HashSet<String>) -> bool {
    excluded.contains(&service.name)
        || signature_references_excluded_type(&service.constructor.params, &service.constructor.return_type, excluded)
        || service
            .configurators
            .iter()
            .any(|method| signature_references_excluded_type(&method.params, &method.return_type, excluded))
        || service.registrations.iter().any(|registration| {
            signature_references_excluded_type(&registration.metadata_params, &registration.return_type, excluded)
                || registration.variants.iter().any(|variant| {
                    variant
                        .signature_params
                        .iter()
                        .any(|param| references_excluded_type(&param.ty, excluded))
                })
        })
        || service
            .entrypoints
            .iter()
            .any(|entrypoint| signature_references_excluded_type(&entrypoint.params, &entrypoint.return_type, excluded))
}

fn api_without_excluded_types(api: &ApiSurface, exclude_types: &HashSet<String>) -> ApiSurface {
    let mut filtered = api.clone();
    filtered
        .services
        .retain(|service| !service_references_excluded_type(service, exclude_types));
    filtered.types.retain(|typ| !exclude_types.contains(&typ.name));
    for typ in &mut filtered.types {
        typ.fields
            .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        typ.methods
            .retain(|method| !signature_references_excluded_type(&method.params, &method.return_type, exclude_types));
    }
    filtered
        .enums
        .retain(|enum_def| !exclude_types.contains(&enum_def.name));
    for enum_def in &mut filtered.enums {
        for variant in &mut enum_def.variants {
            variant
                .fields
                .retain(|field| !references_excluded_type(&field.ty, exclude_types));
        }
    }
    filtered
        .functions
        .retain(|func| !signature_references_excluded_type(&func.params, &func.return_type, exclude_types));
    filtered.errors.retain(|error| !exclude_types.contains(&error.name));
    filtered
}

fn trait_bridge_manages_function(func_name: &str, config: &ResolvedCrateConfig, language: Language) -> bool {
    let language_name = language.to_string();
    config.trait_bridges.iter().any(|bridge| {
        !bridge.exclude_languages.contains(&language_name)
            && (bridge.register_fn.as_deref() == Some(func_name)
                || bridge.unregister_fn.as_deref() == Some(func_name)
                || bridge.clear_fn.as_deref() == Some(func_name))
    })
}

fn api_without_trait_bridge_managed_functions(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    language: Language,
) -> ApiSurface {
    let mut filtered = api.clone();
    filtered
        .functions
        .retain(|func| !trait_bridge_manages_function(&func.name, config, language));
    filtered
}

/// Drop functions named by `[crates.java].exclude_functions` from the surface every Java
/// emitter reads.
///
/// Filtered here rather than inside an emitter because three of them walk `api.functions`
/// independently -- `native_lib.rs`, `ffi_class.rs` (twice) and `facade.rs` -- so hiding the
/// function in one still leaks it from the others.
///
/// Deliberately NOT unioned into `native_lib.rs`'s `ffi_excluded` set: that set marks symbols
/// as absent from the C ABI, whereas a Java-excluded function is still exported by the FFI
/// crate and merely hidden from Java's surface. Conflating the two would declare a present
/// symbol optional.
fn api_without_java_excluded_functions(api: &ApiSurface, excluded: &HashSet<String>) -> ApiSurface {
    let mut filtered = api.clone();
    filtered.functions.retain(|func| !excluded.contains(&func.name));
    filtered
}

impl Backend for JavaBackend {
    fn name(&self) -> &str {
        "java"
    }

    fn language(&self) -> Language {
        Language::Java
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let exclude_types = effective_exclude_types(api, config);
        let filtered_api;
        let api = if exclude_types.is_empty() {
            api
        } else {
            filtered_api = api_without_excluded_types(api, &exclude_types);
            &filtered_api
        };
        let bridge_filtered_api;
        let api = if api
            .functions
            .iter()
            .any(|func| trait_bridge_manages_function(&func.name, config, Language::Java))
        {
            bridge_filtered_api = api_without_trait_bridge_managed_functions(api, config, Language::Java);
            &bridge_filtered_api
        } else {
            api
        };
        let java_excluded: HashSet<String> = config
            .java
            .as_ref()
            .map(|java| java.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let java_filtered_api;
        let api = if java_excluded.is_empty() {
            api
        } else {
            java_filtered_api = api_without_java_excluded_functions(api, &java_excluded);
            &java_filtered_api
        };
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;
        let package = config.java_package();
        let prefix = config.ffi_prefix();
        let main_class = Self::resolve_main_class(api);
        let package_path = package.replace('.', "/");

        let output_dir = config
            .output_for("java")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "packages/java/src/main/java/".to_string());

        let base_path = if output_dir.ends_with(&package_path) || output_dir.ends_with(&format!("{}/", package_path)) {
            PathBuf::from(&output_dir)
        } else {
            PathBuf::from(&output_dir).join(&package_path)
        };

        let java_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> = config
            .java
            .as_ref()
            .map(|c| c.capsule_types.clone())
            .unwrap_or_default();
        crate::core::config::languages::require_shared_native_runtime(
            &java_capsule_types,
            config.java.as_ref().is_some_and(|java| java.shares_native_runtime),
            "java",
        )?;

        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let has_visitor_pattern = crate::backends::java::gen_visitor::has_visitor_generation_metadata(api, config);
        let mut files = Vec::new();

        let description = config
            .scaffold
            .as_ref()
            .and_then(|s| s.description.as_deref())
            .unwrap_or("Generated Java bindings.");
        files.push(GeneratedFile {
            path: base_path.join("package-info.java"),
            content: format!(
                "/**\n * {description}\n */\npackage {package};\n",
                description = description,
                package = package,
            ),
            generated_header: true,
        });

        files.push(GeneratedFile {
            path: base_path.join("NativeLib.java"),
            content: gen_native_lib(api, config, &package, &prefix, has_visitor_pattern),
            generated_header: true,
        });

        files.push(GeneratedFile {
            path: base_path.join(format!("{}.java", main_class)),
            content: gen_main_class(
                api,
                config,
                &package,
                &main_class,
                &prefix,
                &bridge_param_names,
                &bridge_type_aliases,
                has_visitor_pattern,
                &java_capsule_types,
            ),
            generated_header: true,
        });

        files.push(GeneratedFile {
            path: base_path.join(format!("{}Exception.java", main_class)),
            content: gen_exception_class(&package, &main_class),
            generated_header: true,
        });

        for (class_name, code, doc) in [
            (
                "ConversionErrorException",
                crate::core::ir::ApiSurface::FFI_ERROR_CODE_CONVERSION as i32,
                "Exception thrown when an FFI value conversion fails.",
            ),
            (
                "CoreErrorException",
                crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN as i32,
                "Exception thrown when the Rust core reports an unknown error.",
            ),
            (
                "PanicException",
                crate::core::ir::ApiSurface::FFI_ERROR_CODE_PANIC as i32,
                "Exception thrown when a Rust panic is contained at the FFI boundary.",
            ),
        ] {
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.java", class_name)),
                content: gen_infrastructure_exception_class(&package, &main_class, class_name, code, doc),
                generated_header: true,
            });
        }

        // This is used when a struct field has #[serde(default)] and the field type is an enum.
        let enum_defaults = crate::extract::default_value_for_enum::enum_default_variants_map_with_metadata(api);

        let complex_enums: AHashSet<String> = AHashSet::new();

        let sealed_unions_with_unwrapped: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| {
                e.serde_tag.is_some()
                    && e.variants
                        .iter()
                        .any(|v| v.fields.len() == 1 && helpers::is_tuple_field_name(&v.fields[0].name))
            })
            .map(|e| e.name.clone())
            .collect();

        let sealed_interface_names: AHashSet<String> = api
            .enums
            .iter()
            .filter(|e| e.serde_tag.is_some())
            .map(|e| e.name.clone())
            .collect();

        let lang_rename_all = config.serde_rename_all_for_language(Language::Java);

        let visible_type_names: HashSet<&str> = api
            .types
            .iter()
            .filter(|t| !t.is_trait)
            .map(|t| t.name.as_str())
            .chain(api.enums.iter().map(|e| e.name.as_str()))
            .collect();

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            let is_unit_serde = !typ.is_opaque && typ.fields.is_empty() && typ.has_serde;
            if !typ.is_opaque && (!typ.fields.is_empty() || is_unit_serde) {
                let builder_mode = config
                    .java
                    .as_ref()
                    .map(|j| j.dto.builder)
                    .unwrap_or(JavaBuilderMode::Auto);
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", typ.name)),
                    content: gen_record_type(
                        &package,
                        typ,
                        &complex_enums,
                        &sealed_unions_with_unwrapped,
                        &lang_rename_all,
                        &config.trait_bridges,
                        &main_class,
                        builder_mode,
                        &enum_defaults,
                        &sealed_interface_names,
                        &visible_type_names,
                    ),
                    generated_header: true,
                });
            }
        }

        files.push(GeneratedFile {
            path: base_path.join("ByteArraySerializer.java"),
            content: gen_byte_array_serializer(&package),
            generated_header: true,
        });

        files.push(GeneratedFile {
            path: base_path.join("JsonUtil.java"),
            content: gen_json_util_class(&package, &main_class),
            generated_header: true,
        });

        let enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
        let opaque_type_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        let to_json_type_names: AHashSet<String> = api
            .types
            .iter()
            .filter(|t| {
                !t.is_opaque
                    && t.has_serde
                    && !t.name.ends_with("Update")
                    && !t.methods.iter().any(|m| m.name == "to_json")
            })
            .map(|t| t.name.clone())
            .collect();
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", typ.name)),
                    content: gen_opaque_handle_class(
                        &package,
                        typ,
                        &prefix,
                        &config.adapters,
                        &main_class,
                        &enum_names,
                        &opaque_type_names,
                        &to_json_type_names,
                    ),
                    generated_header: true,
                });
            }
        }

        let text_types = &config.untagged_union_text_types;
        for enum_def in &api.enums {
            if has_visitor_pattern
                && config
                    .trait_bridges
                    .iter()
                    .any(|bridge| bridge.result_type.as_deref() == Some(enum_def.name.as_str()))
            {
                continue;
            }
            files.push(GeneratedFile {
                path: base_path.join(format!("{}.java", enum_def.name)),
                content: gen_enum_class(&package, enum_def, &main_class, text_types),
                generated_header: true,
            });
        }

        let infrastructure_exception_names: AHashSet<&str> =
            ["ConversionErrorException", "CoreErrorException", "PanicException"]
                .into_iter()
                .collect();
        let mut emitted_exception_names: AHashSet<String> = AHashSet::new();
        for error in &api.errors {
            for (class_name, content) in crate::codegen::error_gen::gen_java_error_types(error, &package) {
                if infrastructure_exception_names.contains(class_name.as_str()) {
                    continue;
                }
                if !emitted_exception_names.insert(class_name.clone()) {
                    continue;
                }
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}.java", class_name)),
                    content,
                    generated_header: true,
                });
            }
        }

        if has_visitor_pattern {
            for (filename, content) in
                crate::backends::java::gen_visitor::gen_visitor_files(api, config, &package, &main_class)
                    .unwrap_or_default()
            {
                files.push(GeneratedFile {
                    path: base_path.join(filename),
                    content,
                    generated_header: false,
                });
            }
        }

        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.exclude_languages.contains(&Language::Java.to_string()) {
                continue;
            }

            if has_visitor_pattern && bridge_cfg.bind_via == BridgeBinding::OptionsField {
                continue;
            }

            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name && t.is_trait) {
                let has_super_trait = bridge_cfg.super_trait.is_some();
                let trait_bridge::BridgeFiles {
                    interface_content,
                    bridge_content,
                } = trait_bridge::gen_trait_bridge_files(
                    trait_def,
                    &prefix,
                    &package,
                    has_super_trait,
                    bridge_cfg.unregister_fn.as_deref(),
                    bridge_cfg.clear_fn.as_deref(),
                    &visible_type_names,
                    &exclude_types,
                    &bridge_cfg.ffi_skip_methods,
                );

                let adapter_content = trait_bridge::gen_trait_adapter_bridge_file(
                    trait_def,
                    &package,
                    has_super_trait,
                    &visible_type_names,
                    &exclude_types,
                    &bridge_cfg.ffi_skip_methods,
                );

                files.push(GeneratedFile {
                    path: base_path.join(format!("I{}.java", trait_def.name)),
                    content: interface_content,
                    generated_header: true,
                });
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}Bridge.java", trait_def.name)),
                    content: bridge_content,
                    generated_header: true,
                });
                files.push(GeneratedFile {
                    path: base_path.join(format!("{}Adapter.java", trait_def.name)),
                    content: adapter_content,
                    generated_header: true,
                });
            }
        }

        for file in &mut files {
            file.content = line_wrap::wrap_long_java_lines(&file.content);
        }

        Ok(files)
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let exclude_types = effective_exclude_types(api, config);
        let type_filtered_api;
        let api = if exclude_types.is_empty() {
            api
        } else {
            type_filtered_api = api_without_excluded_types(api, &exclude_types);
            &type_filtered_api
        };
        let bridge_filtered_api;
        let api = if api
            .functions
            .iter()
            .any(|func| trait_bridge_manages_function(&func.name, config, Language::Java))
        {
            bridge_filtered_api = api_without_trait_bridge_managed_functions(api, config, Language::Java);
            &bridge_filtered_api
        } else {
            api
        };
        let java_excluded: HashSet<String> = config
            .java
            .as_ref()
            .map(|java| java.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let java_filtered_api;
        let api = if java_excluded.is_empty() {
            api
        } else {
            java_filtered_api = api_without_java_excluded_functions(api, &java_excluded);
            &java_filtered_api
        };
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;

        let package = config.java_package();
        let prefix = config.ffi_prefix();
        let main_class = Self::resolve_main_class(api);
        let package_path = package.replace('.', "/");

        let output_dir = config
            .output_for("java")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "packages/java/src/main/java/".to_string());

        let base_path = if output_dir.ends_with(&package_path) || output_dir.ends_with(&format!("{}/", package_path)) {
            PathBuf::from(&output_dir)
        } else {
            PathBuf::from(&output_dir).join(&package_path)
        };

        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        let has_visitor_pattern = config.ffi.as_ref().map(|f| f.visitor_callbacks).unwrap_or(false)
            || config
                .trait_bridges
                .iter()
                .any(|b| b.bind_via == BridgeBinding::OptionsField);
        let public_class = main_class.trim_end_matches("Rs").to_string();
        let facade_content = gen_facade_class(
            api,
            &package,
            &public_class,
            &main_class,
            &prefix,
            &bridge_param_names,
            &bridge_type_aliases,
            has_visitor_pattern,
            config,
        );

        Ok(vec![GeneratedFile {
            path: base_path.join(format!("{}.java", public_class)),
            content: line_wrap::wrap_long_java_lines(&facade_content),
            generated_header: true,
        }])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let exclude_types = effective_exclude_types(api, config);
        if exclude_types.is_empty() {
            service_api::generate(api, config)
        } else {
            service_api::generate(&api_without_excluded_types(api, &exclude_types), config)
        }
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "mvn",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{FunctionDef, MethodDef, TypeDef};

    #[test]
    fn removes_trait_bridge_managed_functions_from_java_api_functions() {
        let api = ApiSurface {
            functions: vec![
                FunctionDef {
                    name: "register_document_extractor".to_string(),
                    ..FunctionDef::default()
                },
                FunctionDef {
                    name: "unregister_document_extractor".to_string(),
                    ..FunctionDef::default()
                },
                FunctionDef {
                    name: "clear_document_extractors".to_string(),
                    ..FunctionDef::default()
                },
                FunctionDef {
                    name: "list_document_extractors".to_string(),
                    ..FunctionDef::default()
                },
            ],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "DocumentExtractor".to_string(),
                register_fn: Some("register_document_extractor".to_string()),
                unregister_fn: Some("unregister_document_extractor".to_string()),
                clear_fn: Some("clear_document_extractors".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let filtered = api_without_trait_bridge_managed_functions(&api, &config, Language::Java);
        let function_names: Vec<_> = filtered.functions.iter().map(|func| func.name.as_str()).collect();

        assert_eq!(function_names, vec!["list_document_extractors"]);
    }

    #[test]
    fn native_library_omits_free_bytes_without_a_producer() {
        let output = gen_native_lib(
            &ApiSurface::default(),
            &ResolvedCrateConfig::default(),
            "dev.sample",
            "sample",
            false,
        );

        assert!(!output.contains("SAMPLE_FREE_BYTES"));
        assert!(!output.contains("sample_free_bytes"));
    }

    #[test]
    fn native_library_declares_lifecycle_handles_for_every_serializable_type() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "AccessPolicy".into(),
                has_serde: true,
                methods: vec![MethodDef {
                    name: "from_json".into(),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let output = gen_native_lib(&api, &ResolvedCrateConfig::default(), "dev.sample", "sample", false);

        assert!(output.contains("SAMPLE_ACCESS_POLICY_FROM_JSON"));
        assert!(output.contains("sample_access_policy_from_json"));
        assert!(output.contains("SAMPLE_ACCESS_POLICY_FREE"));
        assert!(output.contains("sample_access_policy_free"));
    }

    #[test]
    fn public_api_omits_functions_that_reference_excluded_types() {
        let api = ApiSurface {
            crate_name: "sample".into(),
            types: vec![TypeDef {
                name: "BorrowedView".into(),
                has_lifetime_params: true,
                ..TypeDef::default()
            }],
            functions: vec![FunctionDef {
                name: "get_borrowed_view".into(),
                return_type: TypeRef::Named("BorrowedView".into()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig::default();

        let binding_files = JavaBackend.generate_bindings(&api, &config).expect("Java bindings");
        let public_files = JavaBackend.generate_public_api(&api, &config).expect("Java public API");

        assert!(
            binding_files
                .iter()
                .all(|file| !file.content.contains("getBorrowedView"))
        );
        assert!(
            public_files
                .iter()
                .all(|file| !file.content.contains("getBorrowedView"))
        );
    }

    #[test]
    fn native_library_uses_legacy_visitor_symbols_without_phantom_registry_handles() {
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "MarkupVisitor".to_string(),
                bind_via: BridgeBinding::OptionsField,
                options_type: Some("RenderOptions".to_string()),
                options_field: Some("renderer".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let output = gen_native_lib(&ApiSurface::default(), &config, "dev.sample", "sample", true);

        assert!(output.contains("sample_visitor_create"));
        assert!(output.contains("sample_visitor_free"));
        assert!(!output.contains("sample_register_markup_visitor"));
        assert!(!output.contains("SAMPLE_REGISTER_MARKUP_VISITOR"));
    }

    #[test]
    fn native_library_prioritizes_explicit_paths_and_aggregates_symbol_validation() {
        let api = ApiSurface {
            functions: vec![FunctionDef {
                name: "process_document".to_string(),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };
        let output = gen_native_lib(
            &api,
            &ResolvedCrateConfig {
                name: "sample".to_string(),
                ..ResolvedCrateConfig::default()
            },
            "dev.sample",
            "sample",
            false,
        );

        let explicit_path = output.find("String explicitPath").expect("explicit path branch");
        let resource = output
            .find("tryExtractAndLoadFromResources")
            .expect("bundled resource branch");
        assert!(explicit_path < resource, "explicit path must precede bundled resources");
        assert!(output.contains("SAMPLE_FFI_LIB_PATH"));
        assert!(output.contains("sample_ffi.library.path"));
        assert!(output.contains("sample_process_document"));
        assert!(output.contains("List<String> missing = new ArrayList<>()"));
        assert!(output.contains("exports \"\n                        + exportedCount + \" of \""));
        assert!(output.contains("Loaded from: "));
        assert!(output.contains("validateRequiredSymbols(loadedLibraryPath)"));
    }
}
