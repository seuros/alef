pub(crate) fn emit_lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let package = jni_kotlin_package(config);
    let bridge = bridge_class_name(&config.name);
    let core_crate = core_use_path(config);
    let error_class = resolve_error_class(config, &package);

    let mut out = String::new();

    out.push_str(&template_env::render(
        "lib_header.rs.jinja",
        context! {
            core_crate => core_crate,
            error_class => error_class,
        },
    ));

    for trait_path in collect_trait_imports(api) {
        out.push_str(&format!("use {trait_path};\n"));
    }

    emit_runtime_helpers(&mut out);

    let exclude_functions: std::collections::HashSet<&str> = config
        .kotlin_android
        .as_ref()
        .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
        .unwrap_or_default();

    // Type-level exclusions inherited from the shared `[crates.ffi].exclude_types`
    // plus the paired `[crates.kotlin_android].exclude_types` list, mirroring the
    // kotlin_android binding backend's `effective_exclude_types`. Without this the
    // JNI shims expose opaque client types (and any top-level function whose
    // signature references them) that every other FFI-derived binding drops.
    let exclude_types: std::collections::HashSet<&str> = {
        let mut set: std::collections::HashSet<&str> = config
            .ffi
            .as_ref()
            .map(|ffi| ffi.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();
        if let Some(ka) = config.kotlin_android.as_ref() {
            set.extend(ka.exclude_types.iter().map(String::as_str));
        }
        set
    };
    let signature_references_excluded = |params: &[ParamDef], return_type: &TypeRef| -> bool {
        let references_excluded = |ty: &TypeRef| exclude_types.iter().any(|name| ty.references_named(name));
        references_excluded(return_type) || params.iter().any(|param| references_excluded(&param.ty))
    };

    let trait_bridge_fn_names: std::collections::HashSet<&str> = config
        .trait_bridges
        .iter()
        .flat_map(|b| {
            [&b.register_fn, &b.unregister_fn, &b.clear_fn]
                .into_iter()
                .filter_map(|opt| opt.as_deref())
        })
        .collect();

    // The JNI shims do not emit `#[cfg]` gates per function, so same-named cfg-variant entries
    // (real impl + no-ORT stub fallback) would produce two `Java_*_native…` `#[no_mangle]`
    let deduped_functions = crate::codegen::fn_dedup::dedup_same_name_functions(&api.functions);
    let visible_functions: Vec<_> = deduped_functions
        .iter()
        .filter(|f| {
            !f.sanitized
                && !exclude_functions.contains(f.name.as_str())
                && !trait_bridge_fn_names.contains(f.name.as_str())
                && !signature_references_excluded(&f.params, &f.return_type)
        })
        .collect();

    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    for f in &visible_functions {
        let method_name = bridge_method_name("", &f.name);
        let symbol = jni_symbol(&package, &bridge, &method_name);
        emit_function_shim(
            &mut out,
            &symbol,
            &f.rust_path,
            &f.params,
            &f.return_type,
            f.is_async,
            f.error_type.is_some(),
            &opaque_type_names,
            &config.name,
        );
    }

    let client_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| {
            t.is_opaque
                && !t.is_trait
                && !exclude_types.contains(t.name.as_str())
                && t.methods.iter().any(|m| !m.sanitized && !m.is_static)
        })
        .collect();
    let client_type_names: std::collections::HashSet<&str> = client_types.iter().map(|t| t.name.as_str()).collect();

    for ty in &client_types {
        emit_client_shims(
            &mut out,
            ty,
            api,
            config,
            &package,
            &bridge,
            &exclude_functions,
            &opaque_type_names,
        );
    }

    let top_level_opaque_returns: std::collections::HashSet<&str> = visible_functions
        .iter()
        .filter_map(|f| {
            if let TypeRef::Named(n) = &f.return_type {
                if opaque_type_names.contains(n.as_str()) && !client_type_names.contains(n.as_str()) {
                    return Some(n.as_str());
                }
            }
            None
        })
        .collect();

    for type_name in &top_level_opaque_returns {
        let free_name = destructor_method_name(type_name);
        let free_symbol = jni_symbol(&package, &bridge, &free_name);
        emit_destructor_shim(&mut out, &free_symbol, type_name);
    }

    emit_trait_bridge_shims(&mut out, config, api, &package, &bridge);

    out
}
