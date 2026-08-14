// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

struct JniBridgeInputs<'a> {
    bridge_name: String,
    exception_class: String,
    lib_name: String,
    package: String,
    exclude_functions: std::collections::HashSet<String>,
    visible_functions: Vec<&'a crate::core::ir::FunctionDef>,
    opaque_type_names: std::collections::HashSet<&'a str>,
    capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
}

/// Emit the Kotlin Bridge object containing every JNI `external fun` declaration.
pub fn emit_jni_bridge_object(api: &ApiSurface, config: &ResolvedCrateConfig) -> GeneratedFile {
    let inputs = jni_bridge_inputs(api, config);
    let mut body = template_env::render(
        "jni_bridge_object_header.jinja",
        minijinja::context! {
            bridge_name => inputs.bridge_name,
            lib_name => inputs.lib_name,
        },
    );
    let mut native_names = std::collections::HashSet::new();
    let mut destructor_names = std::collections::HashSet::new();
    emit_top_level_jni_external_funs(&mut body, &inputs, &mut native_names);
    emit_bridge_method_externals(api, &inputs, &mut body, &mut destructor_names);
    emit_value_method_jni_external_funs(&mut body, api, &inputs.exception_class);
    emit_streaming_jni_external_funs(&mut body, config, &inputs.exception_class);
    emit_constructor_jni_external_funs(&mut body, api, config, &inputs.exception_class);
    emit_trait_bridge_jni_external_funs(
        &mut body,
        config,
        &inputs.exception_class,
        &inputs.package,
        &native_names,
    );
    emit_handle_only_destructors(api, &inputs, &mut body, &mut destructor_names);
    render_jni_bridge_file(config, &inputs, body)
}

fn jni_bridge_inputs<'a>(api: &'a ApiSurface, config: &ResolvedCrateConfig) -> JniBridgeInputs<'a> {
    let bridge_name = format!("{}Bridge", to_pascal_case(&config.name));
    let exclude_functions: std::collections::HashSet<String> = config
        .kotlin_android
        .as_ref()
        .map(|android| android.exclude_functions.iter().cloned().collect())
        .unwrap_or_else(|| {
            config
                .kotlin
                .as_ref()
                .map(|kotlin| kotlin.exclude_functions.iter().cloned().collect())
                .unwrap_or_default()
        });
    let visible_functions = api
        .functions
        .iter()
        .filter(|function| {
            !function.sanitized
                && !exclude_functions.contains(function.name.as_str())
                && !trait_bridge_manages_jni_function(function.name.as_str(), config)
        })
        .collect();
    JniBridgeInputs {
        exception_class: format!("{bridge_name}Exception"),
        bridge_name,
        lib_name: config.jni_lib_name(),
        package: jni_kotlin_package(config),
        exclude_functions,
        visible_functions,
        opaque_type_names: opaque_type_names(api),
        capsule_types: kotlin_android_capsule_types(config),
    }
}

fn opaque_type_names(api: &ApiSurface) -> std::collections::HashSet<&str> {
    api.types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait)
        .map(|type_def| type_def.name.as_str())
        .collect()
}

fn kotlin_android_capsule_types(
    config: &ResolvedCrateConfig,
) -> std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> {
    config
        .kotlin_android
        .as_ref()
        .map(|android| android.capsule_types.clone())
        .unwrap_or_default()
}

fn emit_top_level_jni_external_funs(
    body: &mut String,
    inputs: &JniBridgeInputs<'_>,
    native_names: &mut std::collections::HashSet<String>,
) {
    for function in &inputs.visible_functions {
        let native_name = format!("native{}", to_pascal_case(&function.name));
        native_names.insert(native_name.clone());
        let return_type = if is_capsule_function(function, &inputs.capsule_types) {
            "Long"
        } else {
            jni_return_type_for_function(&function.return_type, &inputs.opaque_type_names)
        };
        body.push('\n');
        push_jni_external_fun(
            body,
            &native_name,
            &jni_params_for_function(function, &inputs.opaque_type_names),
            non_unit_return_type(&function.return_type, return_type),
            Some(&inputs.exception_class),
        );
    }
}

fn emit_bridge_method_externals(
    api: &ApiSurface,
    inputs: &JniBridgeInputs<'_>,
    body: &mut String,
    destructor_names: &mut std::collections::HashSet<String>,
) {
    let marker = "// JNI external funs for client instance methods";
    let methods_before = body.matches(marker).count();
    emit_method_jni_external_funs(
        body,
        api,
        &inputs.exclude_functions,
        &inputs.capsule_types,
        &inputs.exception_class,
        destructor_names,
    );
    if methods_before == body.matches(marker).count() {
        emit_fallback_method_externals(api, inputs, body);
    }
}

fn emit_fallback_method_externals(api: &ApiSurface, inputs: &JniBridgeInputs<'_>, body: &mut String) {
    let types = api.types.iter().filter(|type_def| {
        type_def.is_opaque
            && !type_def.is_trait
            && type_def
                .methods
                .iter()
                .any(|method| !method.sanitized && !method.is_static)
            && !inputs
                .exclude_functions
                .iter()
                .all(|excluded| type_def.methods.iter().all(|method| excluded == &method.name))
    });
    let mut emitted_header = false;
    for type_def in types {
        if !emitted_header {
            body.push_str("\n    // JNI external funs for client instance methods (fallback).\n");
            emitted_header = true;
        }
        emit_fallback_type_methods(type_def, inputs, body);
    }
}

fn emit_fallback_type_methods(type_def: &crate::core::ir::TypeDef, inputs: &JniBridgeInputs<'_>, body: &mut String) {
    let owner_pascal = to_pascal_case(&type_def.name);
    for method in &type_def.methods {
        if method.sanitized || method.is_static || inputs.exclude_functions.contains(method.name.as_str()) {
            continue;
        }
        let native_name = format!("native{owner_pascal}{}", to_pascal_case(&method.name));
        let return_type =
            jni_return_type_for_method(&method.return_type, &inputs.opaque_type_names, &inputs.capsule_types);
        push_jni_external_fun(
            body,
            &native_name,
            &method_jni_params(method),
            non_unit_return_type(&method.return_type, return_type),
            Some(&inputs.exception_class),
        );
    }
}

fn emit_handle_only_destructors(
    api: &ApiSurface,
    inputs: &JniBridgeInputs<'_>,
    body: &mut String,
    destructor_names: &mut std::collections::HashSet<String>,
) {
    let client_types: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|type_def| {
            type_def.is_opaque
                && !type_def.is_trait
                && type_def
                    .methods
                    .iter()
                    .any(|method| !method.sanitized && !method.is_static)
        })
        .map(|type_def| type_def.name.as_str())
        .collect();
    let returns = handle_only_return_names(api, inputs, &client_types);
    if returns.is_empty() {
        return;
    }
    body.push_str("\n    // Destructor external funs for handle-only opaque types.\n");
    for type_name in returns {
        let free_name = format!("nativeFree{}", to_pascal_case(type_name));
        if destructor_names.insert(free_name.clone()) {
            push_jni_external_fun(body, &free_name, "handle: Long", None, None);
        }
    }
}

fn handle_only_return_names<'a>(
    api: &'a ApiSurface,
    inputs: &'a JniBridgeInputs<'a>,
    client_types: &std::collections::HashSet<&str>,
) -> std::collections::BTreeSet<&'a str> {
    let function_returns = inputs
        .visible_functions
        .iter()
        .filter_map(|function| named_return_type(&function.return_type));
    function_returns
        .chain(visible_method_return_names(api, &inputs.exclude_functions))
        .filter(|type_name| {
            inputs.opaque_type_names.contains(*type_name)
                && !client_types.contains(*type_name)
                && !inputs.capsule_types.contains_key(*type_name)
        })
        .collect()
}

fn visible_method_return_names<'a>(
    api: &'a ApiSurface,
    excluded_functions: &'a std::collections::HashSet<String>,
) -> impl Iterator<Item = &'a str> {
    api.types
        .iter()
        .filter(|type_def| type_def.is_opaque && !type_def.is_trait && !type_def.binding_excluded)
        .flat_map(|type_def| type_def.methods.iter())
        .filter(|method| !method.sanitized && !method.is_static && !excluded_functions.contains(method.name.as_str()))
        .filter_map(|method| named_return_type(&method.return_type))
}

fn render_jni_bridge_file(
    config: &ResolvedCrateConfig,
    inputs: &JniBridgeInputs<'_>,
    mut body: String,
) -> GeneratedFile {
    body.push_str("}\n");
    let content = template_env::render(
        "jni_bridge_file.jinja",
        minijinja::context! {
            package => inputs.package,
            body => body,
        },
    );
    GeneratedFile {
        path: jni_output_path(config, &format!("{}.kt", inputs.bridge_name)),
        content,
        generated_header: false,
    }
}

fn trait_bridge_manages_jni_function(func_name: &str, config: &ResolvedCrateConfig) -> bool {
    let language_name = if config.kotlin_android.is_some() {
        "kotlin_android"
    } else {
        "kotlin"
    };
    config.trait_bridges.iter().any(|bridge| {
        !bridge.exclude_languages.iter().any(|lang| lang == language_name)
            && (bridge.register_fn.as_deref() == Some(func_name)
                || bridge.unregister_fn.as_deref() == Some(func_name)
                || bridge.clear_fn.as_deref() == Some(func_name))
    })
}
