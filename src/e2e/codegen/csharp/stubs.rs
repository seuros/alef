//! C# e2e test-backend stub emission.

use crate::backends::csharp::trait_bridge::csharp_type_visible_pub;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::e2e::codegen::TestBackendEmission;
use crate::e2e::escape::sanitize_ident;
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

/// Collect every `Named` type name referenced within `ty`, at any nesting depth.
///
/// Scopes [`csharp_type_visible_pub`]'s visibility set to exactly the types a given
/// signature can reference, without needing the full crate-wide type universe: a name
/// is visible if it is referenced here and not present in the caller's excluded set.
pub(super) fn collect_named_types<'a>(ty: &'a crate::core::ir::TypeRef, out: &mut std::collections::HashSet<&'a str>) {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.as_str());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Emit the correct default value for a C# test stub return type.
/// When the original type is non-visible (e.g., HiddenRecord), it's mapped to `string`,
/// so we need to return the appropriate default for the visible type, not the original.
fn emit_csharp_stub_default(
    original_type: &crate::core::ir::TypeRef,
    visible_type: &str,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    visible_type_names: &std::collections::HashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;

    // Check if this type or its inner types are non-visible
    fn contains_non_visible(ty: &TypeRef, visible_type_names: &std::collections::HashSet<&str>) -> bool {
        match ty {
            TypeRef::Named(name) => !visible_type_names.contains(name.as_str()),
            TypeRef::Optional(inner) => contains_non_visible(inner, visible_type_names),
            TypeRef::Vec(inner) => contains_non_visible(inner, visible_type_names),
            TypeRef::Map(k, v) => {
                contains_non_visible(k, visible_type_names) || contains_non_visible(v, visible_type_names)
            }
            _ => false,
        }
    }

    if contains_non_visible(original_type, visible_type_names) {
        // Type contains non-visible parts, map to string default
        if visible_type.contains("?") {
            "null".to_string()
        } else {
            "\"\"".to_string()
        }
    } else if matches!(original_type, TypeRef::Named(_)) {
        format!("default({visible_type})")
    } else {
        // Visible type, use the default logic
        defaults.emit_default(original_type)
    }
}

/// Extract a default value from fixture.input.backend for a stub method.
///
/// Given a method name and fixture, attempts to find the corresponding input
/// value in fixture.input.backend. Returns C#-syntax literals for primitives
/// and complex types. For numeric defaults, emits 1 instead of 0
/// (downstream rejects 0 for counts like dimensions).
fn extract_fixture_default(method_name: &str, fixture: &crate::e2e::fixture::Fixture) -> Option<String> {
    let backend_input = fixture.input.get("backend").and_then(|v| v.as_object())?;

    // Try snake_case first, then the original name.
    let snake_name = method_name.to_snake_case();
    let val = backend_input
        .get(&snake_name)
        .or_else(|| backend_input.get(method_name))?;

    Some(match val {
        serde_json::Value::Number(n) => {
            // For numeric defaults, emit 1 instead of 0 if it's 0
            // (downstream validation rejects 0 for counts like dimensions).
            if let Some(i) = n.as_i64() {
                if i == 0 { "1".to_string() } else { i.to_string() }
            } else if let Some(u) = n.as_u64() {
                if u == 0 { "1".to_string() } else { u.to_string() }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::String(s) => format!("\"{}\"", s),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => return None, // Complex types not supported in fixture defaults
    })
}

/// Emit a single C# stub method body into `out`.
///
/// Used by both the main method loop and the super-trait method section of
/// `emit_test_backend` so both paths share the same formatting logic.
/// `method_cs` is the already-PascalCased method name (caller's responsibility).
fn emit_csharp_stub_method(
    out: &mut String,
    method_cs: &str,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    visible_type_names: &std::collections::HashSet<&str>,
    fixture: &crate::e2e::fixture::Fixture,
) {
    use crate::core::ir::TypeRef;

    // C# trait bridge interfaces expose synchronous methods even though Rust traits are async.
    // The bridge implementation blocks on the async Rust call. So stubs must always be sync
    // (never emit `async Task<T>`). Always use the actual return type. Routed through the same
    // `csharp_type_visible_pub` the production interface uses, so stub signatures cannot drift
    // from the interface they implement. ~keep
    let ret_ty = csharp_type_visible_pub(&method.return_type, visible_type_names);
    // Use the visible type to determine the default value, not the original type
    // (e.g., HiddenRecord → string → "")
    // Try to extract a value from fixture.input.backend first; fall back to language defaults.
    let default_val = extract_fixture_default(&method.name, fixture).unwrap_or_else(|| {
        if method.params.is_empty()
            && matches!(
                method.return_type,
                TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize | crate::core::ir::PrimitiveType::U64)
            )
        {
            // For zero-parameter methods returning usize/u64 (properties), check for known
            // properties that have validation requirements.
            match method.name.to_lowercase().as_str() {
                "dimensions" | "embedding_dimensions" | "model_dimensions" => "1".to_string(),
                _ => emit_csharp_stub_default(&method.return_type, &ret_ty, defaults, visible_type_names),
            }
        } else {
            emit_csharp_stub_default(&method.return_type, &ret_ty, defaults, visible_type_names)
        }
    });

    // Build parameter list using visible types (internal types like HiddenRecord
    // are mapped to string to avoid stub referencing non-public types).
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            format!(
                "{} {}",
                csharp_type_visible_pub(&p.ty, visible_type_names),
                p.name.to_lower_camel_case()
            )
        })
        .collect();
    let param_list = params.join(", ");

    // 8-space indent for method declarations (class body level); the caller's
    // class declaration is at 4-space, and the emitter adds 4 more — giving 8+4=12
    // for methods and 4+4=8 for the class line in the final file.
    // ALWAYS emit sync stubs, regardless of is_async in the Rust trait.
    if matches!(method.return_type, TypeRef::Unit) {
        let _ = writeln!(out, "        public void {method_cs}({param_list}) {{ }}");
    } else if method.params.is_empty() {
        // Zero-parameter methods with non-void return become properties in C#
        let _ = writeln!(out, "        public {ret_ty} {method_cs} {{ get; }} = {default_val};");
    } else {
        let _ = writeln!(out, "        public {ret_ty} {method_cs}({param_list})");
        let _ = writeln!(out, "            => {default_val};");
    }
}

/// Emit a C# test backend stub.
///
/// Generates a nested private class implementing the bridge interface
/// (`I{TraitName}`) with minimal stub methods, then returns a
/// `{TraitName}Bridge.Register(new TestStub_{fixture_id}())` expression
/// as the registration call site.
///
/// Rules:
/// - The stub class name is `TestStub_{sanitized_fixture_id}` where the id
///   has been converted to PascalCase (safe C# identifier).
/// - Super-trait properties (Name, Version) are emitted first with literal values;
///   then lifecycle methods (Initialize, Shutdown) are emitted with default bodies.
/// - Required methods are emitted with return-type defaults produced by `CSharpDefaults`.
/// - Async methods return `Task<T>` and are `async`; sync methods are plain.
/// - Type names come from [`csharp_type_visible_pub`] — the same seam the production
///   `I{TraitName}` interface is generated from — so stub signatures cannot drift from
///   the interface they implement. Non-visible types are NOT referenced in test stubs.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> TestBackendEmission {
    emit_test_backend_with_class_name(
        trait_bridge,
        methods,
        fixture,
        "GeneratedBinding",
        &std::collections::HashSet::new(),
    )
}

pub(super) fn emit_test_backend_with_class_name(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    class_name: &str,
    excluded_types: &std::collections::HashSet<&str>,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;

    let defaults = language_defaults("csharp");

    // Derive a safe C# class identifier from the fixture id.
    let stub_class = format!("TestStub_{}", sanitize_ident(&fixture.id).to_upper_camel_case());

    // Interface name: I{TraitName}, spelled with `csharp_type_name` so it matches the
    // production interface the binding backend emits (`trait_bridge.rs`), including
    // initialism folding (e.g. `UuidPair` -> `UUIDPair`, `XMLBackend` -> `IXMLBackend`).
    let trait_pascal = csharp_type_name(&trait_bridge.trait_name);
    let iface_name = format!("I{trait_pascal}");

    // Scope the visibility set to exactly the Named types these methods reference, minus
    // the caller's excluded (non-public) type names — matching the production interface's
    // `visible_type_names` semantics without needing the full crate-wide type universe.
    let mut referenced_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for method in methods.iter() {
        collect_named_types(&method.return_type, &mut referenced_names);
        for param in &method.params {
            collect_named_types(&param.ty, &mut referenced_names);
        }
    }
    let visible_type_names: std::collections::HashSet<&str> = referenced_names
        .into_iter()
        .filter(|name| !excluded_types.contains(name))
        .collect();

    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let mut setup = String::new();

    // Emit a private nested class declaration. This block will be placed at class scope
    // (not inside any method body) by the caller — the emitter adds 4 more spaces of
    // indentation, so each line here carries a 4-space prefix matching the visitor pattern.
    let _ = writeln!(setup, "    private class {stub_class} : {iface_name}");
    let _ = writeln!(setup, "    {{");

    // Track which super-trait methods we've already emitted to avoid duplication.
    let mut emitted_methods = std::collections::HashSet::new();

    // Super-trait properties and methods: when super_trait is configured, emit
    // the required Name and Version properties, then emit lifecycle methods
    // (initialize, shutdown) and domain-specific methods.
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        // Emit hardcoded Name and Version properties (required by Plugin super-trait)
        let _ = writeln!(setup, "        public string Name => \"{plugin_name}\";");
        let _ = writeln!(setup, "        public string Version => \"1.0.0\";");
        let _ = writeln!(setup);
        // Mark name and version as emitted so they won't be re-emitted as methods
        emitted_methods.insert("name".to_string());
        emitted_methods.insert("version".to_string());

        // Emit super-trait methods (initialize, shutdown) and domain methods
        for method in methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
        {
            let method_cs = to_csharp_name(&method.name);
            emit_csharp_stub_method(&mut setup, &method_cs, method, &*defaults, &visible_type_names, fixture);
            emitted_methods.insert(method.name.clone());
        }
    }

    // All remaining methods (including those with default implementations).
    // Skip super-trait methods already emitted above.
    for method in methods.iter() {
        // Skip methods already emitted.
        if emitted_methods.contains(&method.name) {
            continue;
        }
        let method_cs = to_csharp_name(&method.name);
        emit_csharp_stub_method(&mut setup, &method_cs, method, &*defaults, &visible_type_names, fixture);
    }

    let _ = writeln!(setup, "    }}");

    // Registration expression.
    // Always use the high-level `Bridge.Register(impl)` factory — it handles
    // FFI registration internally. The low-level `Bridge.RegisterXxx(impl)`
    // overloads (derived from reg_fn name) return IntPtr and are not the public API.
    let arg_expr = format!("{}Bridge.Register(new {}())", trait_pascal, stub_class);

    // Teardown: each trait-bridge registration leaks into the host registry and
    // pollutes subsequent tests in the same xUnit test run. Emit a cleanup unregister
    // keyed by the stub's Name property — same value we wrote into the stub above.
    let escaped_plugin_name = plugin_name.replace('\\', "\\\\").replace('"', "\\\"");
    let teardown_block = format!("{class_name}.Unregister{trait_pascal}(\"{escaped_plugin_name}\");");

    TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block,
    }
}
