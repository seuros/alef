//! Regression: the php_ext e2e smoke-app generator must call a symbol the php-ext backend
//! (ext-php-rs `#[php_impl]` facade) actually emits for a crate-level free function.
//!
//! The two generators independently decide how a free function is reachable from generated PHP
//! code, so they can drift apart silently. The concrete drift this pins: the php-ext backend
//! never emits crate-level free functions as global `#[php_function]` items — ext-php-rs's
//! `#[php_impl]` registration derive walks every method in a fixed `impl` block and
//! unconditionally references it by Rust identifier, so free functions are placed as static
//! methods on a namespaced facade class instead (`{namespace}\{Extension}Api::{method}`). The
//! php_ext smoke-app generator used to assume a global `<extension>_<function>()` symbol — a
//! naming convention borrowed from the C-ABI backends that never applies to php-ext — so the
//! generated smoke app probed a function the extension could never provide.
//!
//! The subset assertion is deliberately generic rather than a hard-coded symbol name: it is the
//! check that would have caught this class of bug without knowing which symbol regressed.

use std::collections::BTreeSet;

use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::php_ext::PhpExtCodegen;
use alef::e2e::config::{DependencyMode, E2eConfig};

fn config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A single crate-level free function — the shape that used to reach the smoke app as a
/// probed global function.
fn free_function_api() -> ApiSurface {
    let convert = FunctionDef {
        name: "convert".to_string(),
        rust_path: "sample_lib::convert".to_string(),
        params: vec![ParamDef {
            name: "input".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        return_type: TypeRef::String,
        doc: "Convert input text.".to_string(),
        ..Default::default()
    };

    ApiSurface {
        crate_name: "sample_lib".to_string(),
        version: "0.1.0".to_string(),
        functions: vec![convert],
        ..Default::default()
    }
}

/// Every `Namespace\Class::method` the php-ext backend's generated facade actually exposes.
///
/// Parsed straight out of the real generated `lib.rs`, not reimplemented — a hand-mirrored
/// version of the naming rule would pass even when the generator itself regresses.
fn php_ext_facade_callables(api: &ApiSurface) -> BTreeSet<String> {
    let files = PhpBackend.generate_bindings(api, &config()).unwrap();
    let Some(lib) = files.iter().find(|f| f.path.ends_with("lib.rs")) else {
        return BTreeSet::new();
    };

    let Some(after_struct_attr) = lib.content.split("#[php_class]\n#[php(name = \"").nth(1) else {
        return BTreeSet::new();
    };
    let Some(fq_class_escaped) = after_struct_attr.split("\")]").next() else {
        return BTreeSet::new();
    };
    let fq_class = fq_class_escaped.replace("\\\\", "\\");

    let Some(impl_start) = lib.content.find("#[php_impl]\nimpl ") else {
        return BTreeSet::new();
    };
    let impl_block = &lib.content[impl_start..];

    impl_block
        .split("#[php(name = \"")
        .skip(1)
        .filter_map(|fragment| fragment.split('"').next())
        .map(|method| format!("{fq_class}::{method}"))
        .collect()
}

/// Every `#[php_function]` global-function attribute the php-ext backend declares.
fn php_ext_global_functions(api: &ApiSurface) -> BTreeSet<String> {
    let files = PhpBackend.generate_bindings(api, &config()).unwrap();
    let Some(lib) = files.iter().find(|f| f.path.ends_with("lib.rs")) else {
        return BTreeSet::new();
    };

    lib.content
        .split("#[php_function]\n")
        .skip(1)
        .filter_map(|fragment| fragment.split("fn ").nth(1))
        .map(|fragment| {
            fragment
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .collect()
}

/// The callable the php_ext smoke app's `main.php` actually invokes for the configured call,
/// extracted from the real generator output (`$result = <callable>(...)`).
fn php_ext_smoke_callable(e2e_config: &E2eConfig, config: &ResolvedCrateConfig) -> Option<String> {
    let files = PhpExtCodegen
        .generate(&[], e2e_config, config, &[], &[], &[], &[])
        .unwrap();
    let main = files.iter().find(|f| f.path.ends_with("main.php"))?;

    let after = main.content.split("$result = ").nth(1)?;
    after.split('(').next().map(str::to_string)
}

fn registry_call_config(function_name: &str) -> E2eConfig {
    let mut e2e_config = E2eConfig {
        dep_mode: DependencyMode::Registry,
        ..E2eConfig::default()
    };
    e2e_config.call.function = function_name.to_string();
    e2e_config
}

#[test]
fn php_ext_backend_declares_no_global_function_for_a_free_function() {
    let global_functions = php_ext_global_functions(&free_function_api());

    assert!(
        global_functions.is_empty(),
        "the php-ext backend must never emit a `#[php_function]` global for a crate-level free \
         function -- ext-php-rs's `#[php_impl]` facade owns registration instead; declared: \
         {global_functions:?}"
    );
}

#[test]
fn php_ext_smoke_app_does_not_probe_a_global_function() {
    let e2e_config = registry_call_config("convert");
    let files = PhpExtCodegen
        .generate(&[], &e2e_config, &config(), &[], &[], &[], &[])
        .unwrap();
    let main = files.iter().find(|f| f.path.ends_with("main.php")).unwrap();

    assert!(
        !main.content.contains("function_exists("),
        "the php_ext smoke app must not probe a global function -- the php-ext backend never \
         emits one for a crate-level free function; main.php:\n{}",
        main.content
    );
}

#[test]
fn php_ext_smoke_call_matches_a_symbol_the_facade_actually_exposes() {
    let api = free_function_api();
    let config = config();
    let e2e_config = registry_call_config("convert");

    let facade_callables = php_ext_facade_callables(&api);
    assert!(
        !facade_callables.is_empty(),
        "fixture produced no facade callables from the php-ext backend, so this test would pass \
         vacuously"
    );

    let smoke_callable = php_ext_smoke_callable(&e2e_config, &config)
        .expect("php_ext e2e generator did not emit a configured smoke call");

    assert!(
        facade_callables.contains(&smoke_callable),
        "php_ext smoke app calls `{smoke_callable}`, which the php-ext backend's generated \
         facade class does not expose; facade callables: {facade_callables:?}"
    );
}
