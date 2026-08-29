//! Python e2e test-backend stub emission.

use crate::e2e::codegen::TestBackendEmission;
use crate::e2e::fixture::Fixture;

/// Emit a Python test backend stub for a trait-bridge fixture.
///
/// Generates a duck-typed Python class `_TestStub_<fixture_id>` whose methods
/// return sensible default values. When `super_trait` is set, a `name()` method
/// is emitted returning the fixture's name string extracted from `fixture.input`.
///
/// Python trait bridges use duck typing — no base class or explicit interface
/// inheritance is required. The class just needs to provide the right method
/// signatures that the PyO3 bridge's `register_<trait>` function can call.
///
/// The returned `arg_expr` is `_TestStub_<fixture_id>()` (instantiation without
/// wrapping), which is the form expected by the generated `register_<trait>` call.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &Fixture,
) -> TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::{escape_python, sanitize_ident};
    use std::fmt::Write as FmtWrite;

    let stub_name = format!("_TestStub_{}", sanitize_ident(&fixture.id));
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let defaults = language_defaults("python");

    let mut setup = String::new();

    let _ = writeln!(setup, "class {stub_name}:");

    // Track whether we emitted any method (need `pass` if empty class).
    let mut method_count = 0usize;

    // name() from Plugin super-trait, if configured.
    if trait_bridge.super_trait.is_some() {
        let escaped = escape_python(&backend_name);
        let _ = writeln!(setup, "    def name(self):");
        let _ = writeln!(setup, "        return \"{escaped}\"");
        method_count += 1;
        // initialize() has a Rust default impl but PyO3 calls it unconditionally on
        // every registered plugin object — the Python stub must define it.
        let _ = writeln!(setup, "    def initialize(self):");
        let _ = writeln!(setup, "        pass");
        method_count += 1;
        // shutdown() also has a Rust default impl but PyO3 calls it unconditionally
        // on cleanup — the Python stub must define it.
        let _ = writeln!(setup, "    def shutdown(self):");
        let _ = writeln!(setup, "        pass");
        method_count += 1;
    }

    // Required methods only.
    for method in methods {
        if method.has_default_impl {
            continue;
        }
        // Skip Plugin::name if we already emitted it.
        if trait_bridge.super_trait.is_some() && method.name == "name" {
            continue;
        }
        emit_python_stub_method(&mut setup, method, &*defaults);
        method_count += 1;
    }

    // Emit pass for an empty class body (unlikely but correct).
    if method_count == 0 {
        let _ = writeln!(setup, "    pass");
    }

    let arg_expr = format!("{stub_name}()");

    // Indent the entire class definition by 4 spaces so it sits at function-body
    // scope when the caller embeds it inside a `def test_*():` block.
    let indented_setup = indent_block(&setup, 4);

    // Pytest runs every test in a single python process, so registering a
    // test backend leaks into later tests in the suite. Emit
    // `unregister_<trait>("<backend_name>")` after the call+assertions so the
    // shared global registry is restored: the core's
    // `ensure_<trait>_initialized` self-heal triggers on the next access
    // (registry becomes empty after our unregister) and re-seeds defaults
    // like `tesseract` that smoke tests rely on. Without this teardown,
    // `test_register_ocr_backend_trait_bridge` leaves `test-backend` in the
    // registry and any later OCR fixture (e.g. `test_ocr_image_png`) fails
    // with `OCR backend 'tesseract' not registered`.
    let teardown_block = trait_bridge
        .unregister_fn
        .as_deref()
        .map(|unregister_fn| {
            let escaped = escape_python(&backend_name);
            format!("    {unregister_fn}(\"{escaped}\")\n")
        })
        .unwrap_or_default();

    TestBackendEmission {
        setup_block: indented_setup,
        arg_expr,
        type_imports: Vec::new(),
        teardown_block,
    }
}

/// Indent every non-empty line of `block` by `spaces` spaces.
fn indent_block(block: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    block
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if block.ends_with('\n') { "\n" } else { "" }
}

/// Format a single Python stub method returning the language default for its return type.
fn emit_python_stub_method(
    out: &mut String,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
) {
    use std::fmt::Write as FmtWrite;

    // Build parameter list: `self, _p0, _p1, ...` (unused, hence _ prefix).
    let mut param_parts = vec!["self".to_string()];
    for (i, _) in method.params.iter().enumerate() {
        param_parts.push(format!("_p{i}"));
    }
    let params_str = param_parts.join(", ");

    // Default return expression for the return type.
    // Named types in e2e stubs must return JSON-serialisable values: the PyO3
    // bridge calls the Python method and deserialises the return value from JSON.
    // Returning `TypeName()` would reference a type that is not imported/defined
    // in the generated test file and would cause a NameError at runtime. Return
    // an empty dict `{}` instead — it round-trips cleanly through serde_json.
    //
    // For numeric types in test backends, use a nonzero integer default.
    let default_val = match &method.return_type {
        crate::core::ir::TypeRef::Named(_) => "{}".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => "False".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::F32) => "0.0".to_string(),
        crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::F64) => "0.0".to_string(),
        crate::core::ir::TypeRef::Primitive(_) => "1".to_string(),
        other => defaults.emit_default(other),
    };

    let async_kw = if method.is_async { "async " } else { "" };
    let _ = writeln!(out, "    {async_kw}def {name}({params_str}):", name = method.name);
    let _ = writeln!(out, "        return {default_val}");
}

/// Extract a backend name string from the fixture input JSON.
///
/// See [`super::super::rust::extract_backend_name_from_input`] for the lookup strategy.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object()
                && let Some(s) = inner.get("name").and_then(|v| v.as_str())
            {
                return s.to_string();
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}
