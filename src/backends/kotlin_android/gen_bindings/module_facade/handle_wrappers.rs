use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use crate::backends::kotlin::{emit_kdoc_pub, to_lower_camel, to_pascal_case};
use crate::backends::kotlin_android::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, TypeDef, TypeRef};

use super::super::assemble_kt_content;

pub(super) fn emit_handle_wrappers(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
    bridge_name: &str,
    visible_functions: &[&FunctionDef],
) {
    let client_types: HashSet<&str> = api
        .types
        .iter()
        .filter(|type_def| has_instance_methods(type_def))
        .map(|type_def| type_def.name.as_str())
        .collect();
    let reachable_types = reachable_handle_types(visible_functions);
    let handle_types: BTreeMap<&str, &TypeDef> = api
        .types
        .iter()
        .filter(|type_def| is_handle_type(type_def, &client_types, &reachable_types))
        .map(|type_def| (type_def.name.as_str(), type_def))
        .collect();
    for (class_name, type_def) in handle_types {
        emit_handle_wrapper(
            config,
            kotlin_source_dir,
            package,
            files,
            bridge_name,
            class_name,
            type_def,
        );
    }
}

fn has_instance_methods(type_def: &TypeDef) -> bool {
    type_def.is_opaque
        && !type_def.is_trait
        && type_def
            .methods
            .iter()
            .any(|method| !method.sanitized && !method.is_static)
}

/// Opaque types a caller can actually obtain a handle to: those some visible
/// top-level function returns.
///
/// Mirrors the reachability predicate the sibling JNI shim generator
/// (`backends::jni::gen_shims::top_level::top_level_opaque_returns`) and the Kotlin
/// Bridge destructor emitter (`bridge_object::handle_only_opaque_returns`) already
/// apply, so the three stay in agreement about which types exist on the Kotlin side.
fn reachable_handle_types<'a>(visible_functions: &[&'a FunctionDef]) -> HashSet<&'a str> {
    visible_functions
        .iter()
        .filter_map(|function| match &function.return_type {
            TypeRef::Named(name) => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

/// A handle-only opaque type earns a wrapper class -- and therefore a `close()`
/// calling `nativeFree<TypeName>` -- only when it is reachable. A type that is
/// `is_opaque` but that no visible function returns (xberg's `TokenCounter`: public
/// in Rust, no alef-exposed constructor path) cannot be constructed from Kotlin at
/// all, so its wrapper referenced a `nativeFree<TypeName>` the Bridge object never
/// declares and the native shim never implements -- an `Unresolved reference` at
/// compile time pointing at a class nothing could have instantiated.
fn is_handle_type(type_def: &TypeDef, client_types: &HashSet<&str>, reachable_types: &HashSet<&str>) -> bool {
    type_def.is_opaque
        && !type_def.is_trait
        && !client_types.contains(type_def.name.as_str())
        && reachable_types.contains(type_def.name.as_str())
}

fn emit_handle_wrapper(
    config: &ResolvedCrateConfig,
    kotlin_source_dir: &Path,
    package: &str,
    files: &mut Vec<GeneratedFile>,
    bridge_name: &str,
    class_name: &str,
    type_def: &TypeDef,
) {
    let mut body = String::new();
    let mut imports = BTreeSet::new();
    if !type_def.doc.is_empty() {
        emit_kdoc_pub(&mut body, &type_def.doc, "");
    }
    append_handle_header(&mut body, class_name, bridge_name);
    let adapters = streaming_adapters(config, class_name);
    if !adapters.is_empty() {
        add_streaming_imports(&mut imports);
        append_streaming_mapper(&mut body);
        for adapter in adapters {
            append_streaming_method(&mut body, adapter, class_name, bridge_name);
        }
    }
    body.push_str("}\n");
    files.push(GeneratedFile {
        path: kotlin_source_dir.join(format!("{class_name}.kt")),
        content: assemble_kt_content(package, &imports, &body),
        generated_header: false,
    });
}

fn append_handle_header(body: &mut String, class_name: &str, bridge_name: &str) {
    body.push_str(&template_env::render(
        "handle_wrapper_header.jinja",
        minijinja::context! {
            class_name => class_name,
            bridge_name => bridge_name,
            free_name => format!("nativeFree{}", to_pascal_case(class_name)),
        },
    ));
}

fn streaming_adapters<'a>(config: &'a ResolvedCrateConfig, class_name: &str) -> Vec<&'a AdapterConfig> {
    config
        .adapters
        .iter()
        .filter(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming))
        .filter(|adapter| {
            !adapter
                .skip_languages
                .iter()
                .any(|language| language == "kotlin_android")
        })
        .filter(|adapter| adapter.owner_type.as_deref() == Some(class_name))
        .collect()
}

fn add_streaming_imports(imports: &mut BTreeSet<String>) {
    for import in [
        "import com.fasterxml.jackson.databind.ObjectMapper",
        "import com.fasterxml.jackson.datatype.jdk8.Jdk8Module",
        "import com.fasterxml.jackson.databind.PropertyNamingStrategies",
        "import kotlinx.coroutines.Dispatchers",
        "import kotlinx.coroutines.flow.Flow",
        "import kotlinx.coroutines.flow.callbackFlow",
        "import kotlinx.coroutines.withContext",
        "import kotlinx.coroutines.channels.awaitClose",
    ] {
        imports.insert(import.to_string());
    }
}

fn append_streaming_mapper(body: &mut String) {
    body.push_str(&template_env::render(
        "android_streaming_mapper.jinja",
        minijinja::context! {},
    ));
}

fn append_streaming_method(body: &mut String, adapter: &AdapterConfig, class_name: &str, bridge_name: &str) {
    let owner_pascal = to_pascal_case(class_name);
    let adapter_pascal = to_pascal_case(&adapter.name);
    let first_param_name = adapter
        .params
        .first()
        .map(|param| to_lower_camel(&param.name))
        .unwrap_or_else(|| "request".to_string());
    body.push_str(&template_env::render(
        "android_streaming_method.jinja",
        minijinja::context! {
            method_name => to_lower_camel(&adapter.name),
            params => streaming_params(adapter),
            item_type => adapter.item_type.as_deref().unwrap_or("Any"),
            bridge_name => bridge_name,
            jni_start => format!("native{owner_pascal}{adapter_pascal}Start"),
            jni_next => format!("native{owner_pascal}{adapter_pascal}Next"),
            jni_free => format!("native{owner_pascal}{adapter_pascal}Free"),
            first_param_name => first_param_name,
        },
    ));
}

fn streaming_params(adapter: &AdapterConfig) -> String {
    adapter
        .params
        .iter()
        .map(|param| {
            let simple_type = param.ty.rsplit("::").next().unwrap_or(&param.ty);
            format!("{}: {simple_type}", to_lower_camel(&param.name))
        })
        .collect::<Vec<_>>()
        .join(", ")
}
