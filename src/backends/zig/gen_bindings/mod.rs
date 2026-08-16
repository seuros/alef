use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use heck::AsSnakeCase;
use std::path::PathBuf;

use crate::backends::zig::trait_bridge::emit_trait_bridge;

mod errors;
mod functions;
mod helpers;
mod opaque_handles;
mod service_api;
mod types;

use errors::emit_error_set;
use functions::emit_function;
use helpers::emit_helpers;
use opaque_handles::{emit_opaque_constructor, emit_opaque_handle};
use types::{emit_enum, emit_type};

fn zig_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

pub struct ZigBackend;

fn type_references_excluded(ty: &TypeRef, exclude_types: &std::collections::HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

fn signature_references_excluded(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &std::collections::HashSet<String>,
) -> bool {
    type_references_excluded(return_type, exclude_types)
        || params
            .iter()
            .any(|param| type_references_excluded(&param.ty, exclude_types))
}

impl Backend for ZigBackend {
    fn name(&self) -> &str {
        "zig"
    }

    fn language(&self) -> Language {
        Language::Zig
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: false,
            supports_service_api: true,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        if let Some(zig) = config.zig.as_ref() {
            crate::core::config::languages::require_shared_native_runtime(
                &zig.capsule_types,
                zig.shares_native_runtime,
                "zig",
            )?;
        }
        let api = api.with_deduped_functions();
        crate::codegen::cfg::warn_on_ffi_feature_drift(config, Language::Zig);
        let enabled_features: std::collections::HashSet<&str> = config
            .features_for_language(Language::Zig)
            .iter()
            .map(String::as_str)
            .collect();
        // `@cImport` compiles the C header verbatim and Zig resolves declared externs at
        // comptime/link time — same failure mode as Go's cgo: a function/type/enum the FFI
        // library dropped under `#[cfg(feature = "X")]` is a build-time error, not a graceful
        // fallback. Filtering to what the configured Zig feature set actually satisfies (and
        // dropping cfg-gated fields/variants on surviving types) keeps the generated module
        // consistent with the linked native library, mirroring `with_cfg_filtered_deep`'s
        // existing use in Swift, Kotlin Android, and JNI. ~keep
        let api = api.with_cfg_filtered_deep(&enabled_features);
        let module_name = zig_module_name(&config.name);
        let header = config.ffi_header_name();
        let prefix = config.ffi_prefix();

        let mut exclude_functions: std::collections::HashSet<String> = config
            .zig
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let mut exclude_types: std::collections::HashSet<String> = config
            .ffi
            .as_ref()
            .map(|c| c.exclude_types.iter().cloned().collect())
            .unwrap_or_default();
        if let Some(zig) = &config.zig {
            exclude_types.extend(zig.exclude_types.iter().cloned());
        }
        if let Some(ffi) = &config.ffi {
            exclude_functions.extend(ffi.exclude_functions.iter().cloned());
        }
        exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
        exclude_types.extend(
            config
                .opaque_types
                .iter()
                .filter(|(_, path)| path.contains('<'))
                .map(|(name, _)| name.clone()),
        );

        let type_is_visible = |name: &str| !exclude_types.contains(name);
        let method_is_visible = |method: &crate::core::ir::MethodDef| {
            !signature_references_excluded(&method.params, &method.return_type, &exclude_types)
        };

        let api = if exclude_types.is_empty() {
            api
        } else {
            let mut filtered = api.clone();
            filtered.types.retain(|typ| type_is_visible(&typ.name));
            for typ in &mut filtered.types {
                typ.fields
                    .retain(|field| !type_references_excluded(&field.ty, &exclude_types));
                typ.methods.retain(method_is_visible);
            }
            filtered.enums.retain(|en| !exclude_types.contains(&en.name));
            filtered
                .functions
                .retain(|func| !signature_references_excluded(&func.params, &func.return_type, &exclude_types));
            filtered
        };

        let zig_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
            config.zig.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();
        // Matches both bare `Named(name)` and `Optional(Named(name))` returns — capsule
        // returns share one raw C ABI regardless of IR optionality, see
        // `functions::opaque_type_name_inner` and `emit_function`'s capsule guard. ~keep
        let uses_capsule = !zig_capsule_types.is_empty()
            && api.functions.iter().any(|f| {
                functions::opaque_type_name_inner(&f.return_type).is_some_and(|n| zig_capsule_types.contains_key(n))
            });

        let mut content = String::new();
        content.push_str("// Generated by alef. Do not edit by hand.\n");
        content.push('\n');
        content.push_str("const std = @import(\"std\");\n");
        if uses_capsule {
            let capsule_import_names: std::collections::BTreeSet<&str> = api
                .functions
                .iter()
                .filter_map(|f| {
                    functions::opaque_type_name_inner(&f.return_type).and_then(|n| zig_capsule_types.get(n))
                })
                .filter_map(|cap| crate::core::config::languages::zig_capsule_import_name(&cap.host_type))
                .collect();
            for import_name in capsule_import_names {
                content.push_str(&format!("const {import_name} = @import(\"{import_name}\");\n"));
            }
        }
        content.push_str(&crate::backends::zig::template_env::render(
            "c_import.jinja",
            minijinja::context! {
                header => header,
            },
        ));
        content.push('\n');

        emit_helpers(&prefix, &api.errors, &mut content);
        content.push('\n');

        for bridge in &config.trait_bridges {
            if bridge.exclude_languages.iter().any(|lang| lang == "zig") {
                continue;
            }
            if let Some(alias) = &bridge.type_alias {
                content.push_str(&crate::backends::zig::template_env::render(
                    "trait_bridge_alias.jinja",
                    minijinja::context! {
                        alias => alias,
                    },
                ));
                content.push('\n');
            }
        }

        for error in &api.errors {
            emit_error_set(error, &mut content);
            content.push('\n');
        }

        for ty in api
            .types
            .iter()
            .filter(|t| !exclude_types.contains(&t.name) && !t.is_opaque && t.has_serde)
        {
            emit_type(ty, &mut content);
            content.push('\n');
        }

        for en in api.enums.iter().filter(|e| !exclude_types.contains(&e.name)) {
            emit_enum(en, &mut content);
            content.push('\n');
        }

        let declared_errors: Vec<String> = api.errors.iter().map(|e| e.name.clone()).collect();
        let mut top_level_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for f in &api.functions {
            top_level_names.insert(f.name.clone());
        }
        for ty in &api.types {
            top_level_names.insert(ty.name.clone());
        }
        for en in &api.enums {
            top_level_names.insert(en.name.clone());
        }
        let struct_names: std::collections::HashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde)
            .map(|t| t.name.clone())
            .collect();
        let enum_names: std::collections::HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
        let opaque_creator_map: std::collections::HashMap<String, (String, String)> = {
            let mut map = std::collections::HashMap::new();
            for opaque_ty in api
                .types
                .iter()
                .filter(|t| !t.is_trait && (t.is_opaque || !t.has_serde))
            {
                if let Some(creator) = api
                    .functions
                    .iter()
                    .find(|f| matches!(&f.return_type, crate::core::ir::TypeRef::Named(n) if n == &opaque_ty.name))
                    && let Some(config_param) = creator.params.first()
                    && let Some(config_name) = functions::opaque_type_name_inner(&config_param.ty)
                {
                    map.insert(
                        opaque_ty.name.clone(),
                        (creator.name.clone(), heck::AsSnakeCase(config_name).to_string()),
                    );
                }
            }
            map
        };

        let trait_bridge_fn_names: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|lang| lang == "zig"))
            .flat_map(|b| {
                let mut names = Vec::new();
                if let Some(trait_def) = api.types.iter().find(|t| t.name == b.trait_name && t.is_trait) {
                    let snake = heck::AsSnakeCase(&trait_def.name).to_string();
                    names.push(format!("register_{snake}"));
                    names.push(format!("unregister_{snake}"));
                }
                if let Some(clear_fn) = b.clear_fn.as_deref() {
                    names.push(clear_fn.to_string());
                }
                names
            })
            .collect();
        for f in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
            if trait_bridge_fn_names.contains(&f.name) {
                continue;
            }
            emit_function(
                f,
                &prefix,
                &declared_errors,
                &top_level_names,
                &struct_names,
                &opaque_creator_map,
                &zig_capsule_types,
                &mut content,
            );
            content.push('\n');
        }

        let error_type = config.error_type.as_deref().unwrap_or("error");
        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.exclude_languages.iter().any(|lang| lang == "zig") {
                continue;
            }
            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name && t.is_trait) {
                emit_trait_bridge(&prefix, error_type, bridge_cfg, trait_def, &exclude_types, &mut content);
                content.push('\n');
            } else {
                let snake = AsSnakeCase(&bridge_cfg.trait_name).to_string();
                return Err(anyhow::anyhow!(
                    "zig backend: trait bridge '{}' has no trait definition in binding surface. \
                    Vtable builders (e.g., make_{}_vtable) will be undefined, breaking e2e tests. \
                    Check that the trait is not in exclude_types or marked binding_excluded.",
                    bridge_cfg.trait_name,
                    snake
                ));
            }
        }

        let streaming_item_types: std::collections::HashMap<String, String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .filter_map(|a| a.item_type.as_ref().map(|item| (a.name.clone(), item.clone())))
            .collect();

        let trait_bridge_type_aliases: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|lang| lang == "zig"))
            .filter_map(|b| b.type_alias.clone())
            .collect();

        for ty in api
            .types
            .iter()
            .filter(|t| !t.is_trait && (t.is_opaque || !t.has_serde))
            .filter(|t| !exclude_types.contains(&t.name))
            .filter(|t| !trait_bridge_type_aliases.contains(&t.name))
        {
            emit_opaque_handle(
                ty,
                &prefix,
                &declared_errors,
                &struct_names,
                &streaming_item_types,
                &enum_names,
                &mut content,
            );
            content.push('\n');
            if let Some(ctor) = config.client_constructors.get(&ty.name) {
                emit_opaque_constructor(ty, &prefix, ctor, &top_level_names, &mut content);
                content.push('\n');
            }
        }

        let dir = resolve_output_dir(None, &config.name, "packages/zig/src");
        let path = PathBuf::from(dir).join(format!("{module_name}.zig"));

        Ok(vec![GeneratedFile {
            path,
            content,
            generated_header: false,
        }])
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let enabled_features: std::collections::HashSet<&str> = config
            .features_for_language(Language::Zig)
            .iter()
            .map(String::as_str)
            .collect();
        let filtered_api = api.with_cfg_filtered_deep(&enabled_features);
        service_api::generate(&filtered_api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "zig",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}
