//! `all_result_enum_classes` (see `render.rs`) is derived from `result_enum_fields` config alone,
//! never from what an assertion body actually names -- unlike `referenced_enums`, which
//! `builders::enum_member_reference` fills as the request-side builders emit each
//! `EnumType.Member` reference. That asymmetry only stays safe if no TypeScript/WASM assertion
//! path ever emits a class-name reference for a result-side enum field.
//!
//! This file pins that invariant rather than refactoring `all_result_enum_classes` into the
//! emitter-recorded pattern: `render_wasm_enum_assertion` (see `assertions.rs`) compares the
//! field against the plain wire-format string (`expect(result.kind).toBe("uri")`), not
//! `EnumClass.Variant`, and its `enum_class` parameter is intentionally unused
//! (`_enum_class: &str`). There is therefore no body reference a config-only derivation could
//! ever fail to cover here -- config-derived or hypothetically IR-derived -- because the
//! renderer that would produce one does not exist on this path. If a future change makes an
//! assertion reference the class by name, it must route through `referenced_enums` the way the
//! request-side builders do; this test exists to fail loudly if that ever happens silently. ~keep

use super::*;

const PKG: &str = "@sample/wasm";
const WASM_TYPE_PREFIX: &str = "Wasm";
const ENUM_CLASS: &str = "WasmInputKind";

fn equals_fixture(id: &str, field: &str, value: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("document".to_string()),
        description: "process a document".to_string(),
        input: serde_json::json!({}),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The identifiers the generated file imports from the binding package.
fn binding_imports(output: &str) -> Vec<String> {
    let line = output
        .lines()
        .find(|line| line.starts_with("import") && line.contains(PKG))
        .unwrap_or_else(|| panic!("generated test must import its bindings:\n{output}"));
    let open = line.find("{ ").expect("binding import must be a named-import list");
    let close = line.rfind(" }").expect("binding import must be a named-import list");
    line[open + 2..close]
        .split(", ")
        .map(|entry| entry.trim().to_string())
        .collect()
}

/// The single rendered `expect(...)` line for an `equals` assertion, so the assertion below
/// pins the exact fact (what the body says) rather than its surrounding whitespace.
fn rendered_expect_line(output: &str) -> &str {
    output
        .lines()
        .find(|line| line.trim_start().starts_with("expect("))
        .unwrap_or_else(|| panic!("generated test must render an expect(...) assertion:\n{output}"))
        .trim()
}

/// A `result_enum_fields`-configured field, asserted with `equals`, renders as a plain
/// wire-string comparison -- never as `WasmInputKind.Something` -- while the class is still
/// imported from config, exactly as `render.rs` renders it today. Both facts must hold at once:
/// the import proves `all_result_enum_classes` still runs, and the assertion line proves it never
/// had a body reference to be honest about.
#[test]
fn wasm_result_enum_field_assertion_never_references_the_configured_class() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "processDocument".to_string();
    e2e_config.call.overrides.insert(
        "wasm".to_string(),
        crate::e2e::config::CallOverride {
            result_enum_fields: [("kind".to_string(), ENUM_CLASS.to_string())].into_iter().collect(),
            ..Default::default()
        },
    );
    let fixture = equals_fixture("process_document", "kind", "uri");

    let output = render_test_file(
        "wasm",
        "document",
        &[&fixture],
        "",
        PKG,
        "processDocument",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        WASM_TYPE_PREFIX,
        &Default::default(),
        &[],
    );

    assert_eq!(
        rendered_expect_line(&output),
        "expect(result.kind).toBe(\"uri\");",
        "a result_enum_fields-configured field must render as a plain wire-string comparison, \
         not an EnumClass.Variant reference:\n{output}"
    );
    assert!(
        binding_imports(&output)
            .iter()
            .any(|identifier| identifier == ENUM_CLASS),
        "the configured class is still imported from `result_enum_fields` config even though \
         nothing in the body names it -- that is the existing, preserved behaviour; got imports \
         {:?}",
        binding_imports(&output)
    );
}

/// Control: with no `result_enum_fields` entry for the asserted field, nothing pulls the enum
/// class into the import list -- `all_result_enum_classes` only ever contains what config names,
/// never a class inferred from the assertion or the IR.
#[test]
fn wasm_import_list_omits_an_enum_class_with_no_result_enum_fields_entry() {
    let e2e_config = {
        let mut config = E2eConfig::default();
        config.call.function = "processDocument".to_string();
        config
    };
    let fixture = equals_fixture("process_document", "kind", "uri");

    let output = render_test_file(
        "wasm",
        "document",
        &[&fixture],
        "",
        PKG,
        "processDocument",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        WASM_TYPE_PREFIX,
        &Default::default(),
        &[],
    );

    assert_eq!(
        binding_imports(&output),
        vec!["processDocument".to_string()],
        "with no result_enum_fields entry, only the called function should be imported:\n{output}"
    );
}
