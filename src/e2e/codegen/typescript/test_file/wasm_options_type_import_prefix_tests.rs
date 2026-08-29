//! A `json_object` "config" argument resolves its constructor type from `options_type`, not
//! from `arg.element_type` — see `json_object_constructor_type`'s `arg.name == "config"` special
//! case. The body (`args.rs`) resolves that constructor reference through
//! `wasm_prefixed_wrapped_type(lang, canonical_ts_type_name(lang, options_type, ..), ..)`, but
//! `render_test_file`'s import-set collection (`render.rs`) used to insert the bare
//! `canonical_ts_type_name` result straight into `all_options_types`, skipping the
//! `wasm_prefixed_wrapped_type` step entirely. The generated file then imported the unprefixed
//! IR name while the body called `<Prefix><Name>.default()` — a `ReferenceError` at runtime that
//! `tsc` cannot catch, because both names are individually valid identifiers.
//!
//! Split out of `tests.rs` (a remediation target at the 1,000-line cap, pinned by
//! `tests/file_size_baseline.txt`, and must not grow) rather than added there. ~keep

use super::tests::{make_field, make_type};
use super::*;
use crate::core::ir::{PrimitiveType, TypeRef};

/// One `ProcessConfig`-shaped type with a single scalar field, wrapped by the wasm-bindgen
/// backend under the `Wasm` prefix.
fn process_config_type_defs() -> Vec<TypeDef> {
    vec![make_type(
        "ProcessConfig",
        vec![make_field("threshold", TypeRef::Primitive(PrimitiveType::U32))],
    )]
}

fn process_call_args() -> Vec<ArgMapping> {
    vec![ArgMapping {
        name: "config".to_string(),
        field: "config".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }]
}

fn process_fixture() -> Fixture {
    Fixture {
        id: "process_with_config".to_string(),
        category: Some("process".to_string()),
        description: "process with a config object".to_string(),
        input: serde_json::json!({ "config": { "threshold": 5 } }),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Fixture::default()
    }
}

/// The single `import { .. } from ".."` line naming the binding package, or a panic with the
/// full output for debugging — every scenario below is expected to render exactly one.
fn binding_import_line<'a>(output: &'a str, pkg_name: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.starts_with("import") && line.contains(pkg_name))
        .unwrap_or_else(|| panic!("must render a binding import line, got:\n{output}"))
}

/// The regression this file exists to pin: a `config` arg's constructor type comes from the
/// call's `options_type`, resolved as the bare IR name (`ProcessConfig`) in `alef.toml`. The
/// wasm-bindgen backend exports it under the `Wasm` prefix, so the body must (and does) call
/// `WasmProcessConfig.default()`. The import line must name the same prefixed class, not the
/// bare IR name the body never references as a bare identifier.
#[test]
fn wasm_options_type_import_matches_the_prefixed_constructor_the_body_calls() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.args = process_call_args();
    let fixture = process_fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "wasm",
        "process",
        &[&fixture],
        "",
        "@test/wasm",
        "process",
        &e2e_config.call.args,
        Some("ProcessConfig"),
        None,
        &e2e_config,
        &process_config_type_defs(),
        &[],
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert!(
        output.contains("WasmProcessConfig.default()"),
        "constructor reference must use the prefixed class;\n{output}"
    );
    assert_eq!(
        binding_import_line(&output, "@test/wasm"),
        "import { process, WasmProcessConfig } from \"@test/wasm\";",
        "the import must name the same prefixed class the body constructs, full output:\n{output}"
    );
}

/// Positive control proving the assertion above can actually distinguish "correct" from "not
/// examined": when `options_type` already names the prefixed wasm class directly (as one
/// consumer repo's `alef.toml` does), `wasm_prefixed_wrapped_type`'s no-double-prefix guard must
/// still produce exactly one `Wasm` prefix, matching the body byte-for-byte. If this test and the
/// one above ever assert the same exact string via two unrelated inputs, both are proof the
/// import line reflects the real resolver rather than a fixed literal an implementation could
/// satisfy without reading `options_type` at all.
#[test]
fn wasm_options_type_import_does_not_double_prefix_an_already_prefixed_options_type() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.args = process_call_args();
    let fixture = process_fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "wasm",
        "process",
        &[&fixture],
        "",
        "@test/wasm",
        "process",
        &e2e_config.call.args,
        Some("WasmProcessConfig"),
        None,
        &e2e_config,
        &process_config_type_defs(),
        &[],
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert_eq!(
        binding_import_line(&output, "@test/wasm"),
        "import { process, WasmProcessConfig } from \"@test/wasm\";",
        "an already-prefixed options_type must not gain a second `Wasm` prefix, full output:\n{output}"
    );
}

/// The same bug, reached through a per-fixture call override's `options_type` instead of the
/// file-level default — `render.rs` collects that value at a second, separate call site
/// (`cc.overrides.get(lang).options_type`) that must apply the identical resolver.
#[test]
fn wasm_per_call_override_options_type_import_matches_the_prefixed_constructor() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.args = process_call_args();
    e2e_config.call.overrides.insert(
        "wasm".to_string(),
        crate::e2e::config::CallOverride {
            options_type: Some("ProcessConfig".to_string()),
            ..Default::default()
        },
    );
    let fixture = process_fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "wasm",
        "process",
        &[&fixture],
        "",
        "@test/wasm",
        "process",
        &e2e_config.call.args,
        None,
        None,
        &e2e_config,
        &process_config_type_defs(),
        &[],
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert!(
        output.contains("WasmProcessConfig.default()"),
        "constructor reference must use the prefixed class;\n{output}"
    );
    assert_eq!(
        binding_import_line(&output, "@test/wasm"),
        "import { process, WasmProcessConfig } from \"@test/wasm\";",
        "the per-call-override options_type must resolve through the same prefix rule, full output:\n{output}"
    );
}

/// Negative control: the node backend treats `options_type` as a TypeScript interface name, not
/// a wasm-bindgen class — `wasm_prefixed_wrapped_type` must stay a no-op there. Pinning this
/// keeps the wasm fix from leaking a `Wasm` prefix into node's `.d.ts`-shaped `type` import.
#[test]
fn node_options_type_import_keeps_the_bare_ir_name() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.args = process_call_args();
    let fixture = process_fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "node",
        "process",
        &[&fixture],
        "",
        "@test/node",
        "process",
        &e2e_config.call.args,
        Some("ProcessConfig"),
        None,
        &e2e_config,
        &process_config_type_defs(),
        &[],
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert_eq!(
        binding_import_line(&output, "@test/node"),
        "import { process, type ProcessConfig } from \"@test/node\";",
        "node's options_type import must stay the bare, unprefixed IR name, full output:\n{output}"
    );
}
