pub(crate) fn emit_lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = jni_kotlin_package(config);
    let bridge = bridge_class_name(&config.name);
    let filtered_api = filtered_jni_api(api, config);
    let excluded_functions = jni_excluded_functions(config);
    let excluded_types = jni_excluded_types(config);
    let trait_bridge_functions = jni_trait_bridge_function_names(config);
    let visible_functions = visible_jni_functions(
        api,
        &filtered_api,
        config,
        &excluded_functions,
        &excluded_types,
        &trait_bridge_functions,
    );
    let opaque_types = jni_opaque_type_names(&filtered_api);
    let capsule_types = jni_capsule_types(config);
    let mut out = emit_jni_lib_header(&filtered_api, config, &package);
    emit_top_level_function_shims(
        &mut out,
        &visible_functions,
        config,
        &package,
        &bridge,
        &opaque_types,
        &capsule_types,
    );
    emit_jni_type_shims(
        &mut out,
        &filtered_api,
        config,
        &excluded_functions,
        &excluded_types,
        &opaque_types,
        &capsule_types,
        &package,
        &bridge,
    );
    emit_trait_bridge_shims(&mut out, config, &filtered_api, &package, &bridge);
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_jni_type_shims(
    out: &mut String,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    excluded_functions: &std::collections::HashSet<&str>,
    excluded_types: &std::collections::HashSet<&str>,
    opaque_types: &std::collections::HashSet<&str>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
    package: &str,
    bridge: &str,
) {
    let client_types = jni_client_types(api, config, excluded_types);
    emit_jni_client_type_shims(out, &client_types, api, config, package, bridge, excluded_functions);
    emit_jni_value_type_shims(out, api, excluded_types, package, bridge);
    emit_opaque_return_destructors(
        out,
        &client_types,
        api,
        config,
        opaque_types,
        capsule_types,
        package,
        bridge,
    );
}

fn filtered_jni_api(api: &ApiSurface, config: &ResolvedCrateConfig) -> ApiSurface {
    let enabled_features = config
        .features_for_language(Language::KotlinAndroid)
        .iter()
        .map(String::as_str)
        .collect();
    api.with_cfg_filtered_deep(&enabled_features)
}

fn emit_jni_lib_header(api: &ApiSurface, config: &ResolvedCrateConfig, package: &str) -> String {
    let mut out = template_env::render(
        "lib_header.rs.jinja",
        context! {
            core_crate => core_use_path(config),
            error_class => resolve_error_class(config, package),
            crate_attributes => crate::codegen::shared::format_crate_attributes(&config.crate_attributes),
        },
    );
    for trait_path in collect_trait_imports(api) {
        out.push_str(&format!("use {trait_path};\n"));
    }
    emit_runtime_helpers(&mut out);
    out
}

fn jni_excluded_functions(config: &ResolvedCrateConfig) -> std::collections::HashSet<&str> {
    let mut excluded: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|android| android.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();
    if let Some(kotlin) = config.kotlin.as_ref() {
        excluded.extend(kotlin.exclude_functions.iter().map(String::as_str));
    }
    if let Some(jni) = config.jni.as_ref() {
        excluded.extend(jni.exclude_functions.iter().map(String::as_str));
    }
    excluded
}

fn jni_excluded_types(config: &ResolvedCrateConfig) -> std::collections::HashSet<&str> {
    let mut excluded: std::collections::HashSet<&str> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().map(String::as_str).collect())
        .unwrap_or_default();
    if let Some(android) = config.kotlin_android.as_ref() {
        excluded.extend(android.exclude_types.iter().map(String::as_str));
    }
    excluded
}

fn jni_trait_bridge_function_names(config: &ResolvedCrateConfig) -> std::collections::HashSet<&str> {
    config
        .trait_bridges
        .iter()
        .flat_map(|bridge| {
            [&bridge.register_fn, &bridge.unregister_fn, &bridge.clear_fn]
                .into_iter()
                .filter_map(|name| name.as_deref())
        })
        .collect()
}

fn visible_jni_functions(
    api: &ApiSurface,
    filtered_api: &ApiSurface,
    config: &ResolvedCrateConfig,
    excluded_functions: &std::collections::HashSet<&str>,
    excluded_types: &std::collections::HashSet<&str>,
    trait_bridge_functions: &std::collections::HashSet<&str>,
) -> Vec<crate::core::ir::FunctionDef> {
    let deduped = crate::codegen::fn_dedup::dedup_same_name_functions(&filtered_api.functions);
    let candidates = if jni_target_overrides(config).is_empty() {
        deduped
    } else {
        api.functions.clone()
    };
    candidates
        .into_iter()
        .filter(|function| {
            !function.sanitized
                && !excluded_functions.contains(function.name.as_str())
                && !trait_bridge_functions.contains(function.name.as_str())
                && !jni_signature_references_excluded(function, excluded_types)
        })
        .collect()
}

fn jni_signature_references_excluded(
    function: &crate::core::ir::FunctionDef,
    excluded_types: &std::collections::HashSet<&str>,
) -> bool {
    let references_excluded = |type_ref: &TypeRef| {
        excluded_types
            .iter()
            .any(|type_name| type_ref.references_named(type_name))
    };
    references_excluded(&function.return_type) || function.params.iter().any(|param| references_excluded(&param.ty))
}

/// The capsule types the JNI shim actually emits `tree_sitter::ffi`-style raw-pointer
/// casts for: the intersection of `[crates.ffi.capsule_types]` (which carries the Rust
/// `into_raw_type`/`package` info) and `[crates.kotlin_android.capsule_types]` (which
/// gates whether the paired Kotlin binding wants a capsule at all). `scaffold_jni` calls
/// this same function to decide which capsule crates to add to the JNI manifest, so the
/// emitted casts and the declared dependencies can never drift apart. ~keep
pub(crate) fn jni_capsule_types(
    config: &ResolvedCrateConfig,
) -> std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig> {
    let Some(android) = config.kotlin_android.as_ref() else {
        return std::collections::HashMap::new();
    };
    config
        .ffi
        .as_ref()
        .into_iter()
        .flat_map(|ffi| ffi.capsule_types.iter())
        .filter(|(type_name, _)| android.capsule_types.contains_key(*type_name))
        .map(|(type_name, capsule)| (type_name.clone(), capsule.clone()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn emit_top_level_function_shims(
    out: &mut String,
    functions: &[crate::core::ir::FunctionDef],
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
    opaque_types: &std::collections::HashSet<&str>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) {
    // The symbol is keyed by its resolved predicate because duplicate IR entries can represent the same re-export. ~keep
    let mut emitted_native_symbols: std::collections::HashSet<(String, Option<String>)> =
        std::collections::HashSet::new();
    for function in functions {
        let Some(target_predicate) = jni_target_predicate(function.cfg.as_deref(), config) else {
            continue;
        };
        let method_name = bridge_method_name("", &function.name);
        if !emitted_native_symbols.insert((method_name.clone(), target_predicate.clone())) {
            continue;
        }
        if let Some(predicate) = target_predicate {
            out.push_str(&template_env::render(
                "cfg_attribute.rs.jinja",
                context! { predicate => predicate },
            ));
        }
        let symbol = jni_symbol(package, bridge, &method_name);
        emit_function_shim(out, &symbol, function, opaque_types, capsule_types, &config.name);
    }
}

fn jni_client_types<'a>(
    api: &'a ApiSurface,
    config: &ResolvedCrateConfig,
    excluded_types: &std::collections::HashSet<&str>,
) -> Vec<&'a TypeDef> {
    let streaming_owners: std::collections::HashSet<&str> = config
        .adapters
        .iter()
        .filter(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming))
        .filter_map(|adapter| adapter.owner_type.as_deref())
        .collect();
    api.types
        .iter()
        .filter(|type_def| {
            type_def.is_opaque
                && !type_def.is_trait
                && !excluded_types.contains(type_def.name.as_str())
                && (type_def
                    .methods
                    .iter()
                    .any(|method| !method.sanitized && !method.is_static)
                    || streaming_owners.contains(type_def.name.as_str()))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn emit_jni_client_type_shims(
    out: &mut String,
    client_types: &[&TypeDef],
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    bridge: &str,
    excluded_functions: &std::collections::HashSet<&str>,
) {
    let opaque_types = jni_opaque_type_names(api);
    for type_def in client_types {
        emit_client_shims(
            out,
            type_def,
            api,
            config,
            package,
            bridge,
            excluded_functions,
            &opaque_types,
        );
    }
}

fn emit_jni_value_type_shims(
    out: &mut String,
    api: &ApiSurface,
    excluded_types: &std::collections::HashSet<&str>,
    package: &str,
    bridge: &str,
) {
    let serde_types = value_bridge_serde_type_names(api);
    for type_def in api.types.iter().filter(|type_def| {
        !type_def.is_opaque
            && !type_def.is_trait
            && !type_def.binding_excluded
            && !excluded_types.contains(type_def.name.as_str())
    }) {
        emit_value_type_shims(out, type_def, package, bridge, &serde_types);
    }
}

/// Destructor shims for opaque types that are reachable from Kotlin but never became a
/// "client type" (`jni_client_types`, which already gets a destructor via
/// [`emit_client_lifecycle_shims`]).
///
/// Reachability is computed by the *same* [`crate::backends::kotlin::handle_only_type_names`]
/// predicate the kotlin_android Bridge object and handle-wrapper emitters use, fed with
/// [`crate::backends::kotlin::kotlin_visible_functions`] /
/// [`crate::backends::kotlin::kotlin_exclude_functions`] rather than this backend's own
/// `visible_jni_functions` -- deliberately, so that a function excluded only via
/// `[crates.jni].exclude_functions` (which tells this backend to skip generating that one
/// function's *own* native shim, not to hide it from Kotlin) does not also drop the destructor
/// for whatever opaque type it returns. Kotlin keeps calling the function and keeps needing to
/// free what it returns either way. `client_types` stays this backend's own (broader) notion of
/// "already has a destructor" -- it also covers streaming-adapter owners with no instance
/// methods, which get their `nativeFree<Type>` from [`emit_client_lifecycle_shims`] instead --
/// so nothing here ever emits a duplicate `#[no_mangle]` symbol. ~keep
#[allow(clippy::too_many_arguments)]
fn emit_opaque_return_destructors(
    out: &mut String,
    client_types: &[&TypeDef],
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    opaque_types: &std::collections::HashSet<&str>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
    package: &str,
    bridge: &str,
) {
    let client_names = client_types.iter().map(|type_def| type_def.name.as_str()).collect();
    let visible_functions = crate::backends::kotlin::kotlin_visible_functions(api, config);
    let exclude_functions = crate::backends::kotlin::kotlin_exclude_functions(config);
    let capsule_type_names: std::collections::HashSet<&str> = capsule_types.keys().map(String::as_str).collect();
    let return_names = crate::backends::kotlin::handle_only_type_names(
        api,
        &visible_functions,
        &exclude_functions,
        opaque_types,
        &capsule_type_names,
        &client_names,
    );
    for type_name in &return_names {
        let symbol = jni_symbol(package, bridge, &destructor_method_name(type_name));
        emit_destructor_shim(out, &symbol, type_name);
    }
}

fn jni_opaque_type_names(api: &ApiSurface) -> std::collections::HashSet<&str> {
    api.types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait)
        .map(|type_def| type_def.name.as_str())
        .collect()
}

fn jni_target_overrides(config: &ResolvedCrateConfig) -> &[crate::core::config::FfiTargetDepOverride] {
    config
        .jni
        .as_ref()
        .map(|jni| jni.target_dep_overrides.as_slice())
        .unwrap_or_default()
}

fn jni_target_predicate(cfg: Option<&str>, config: &ResolvedCrateConfig) -> Option<Option<String>> {
    let overrides = jni_target_overrides(config);
    let default_features: std::collections::HashSet<&str> = config
        .features_for_language(Language::KotlinAndroid)
        .iter()
        .map(String::as_str)
        .collect();
    if overrides.is_empty() {
        return crate::core::ir::cfg_feature_satisfied(cfg, &default_features).then_some(None);
    }

    let mut enabled_predicates = Vec::new();
    if crate::core::ir::cfg_feature_satisfied(cfg, &default_features) {
        let override_predicates = overrides.iter().map(|target| target.cfg.as_str()).collect::<Vec<_>>();
        enabled_predicates.push(format!("not(any({}))", override_predicates.join(", ")));
    }
    for target in overrides {
        let features = target.features.iter().map(String::as_str).collect();
        if crate::core::ir::cfg_feature_satisfied(cfg, &features) {
            enabled_predicates.push(target.cfg.clone());
        }
    }

    match enabled_predicates.len() {
        0 => None,
        count if count == overrides.len() + 1 => Some(None),
        1 => Some(enabled_predicates.pop()),
        _ => Some(Some(format!("any({})", enabled_predicates.join(", ")))),
    }
}
