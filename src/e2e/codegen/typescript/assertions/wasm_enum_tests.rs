//! What the wasm binding actually hands JavaScript for an enum-typed struct field, and what the
//! assertion must therefore compare against.
//!
//! ~keep Both defects covered here are FALSE FAILURES rather than false passes: a correctly
//! authored fixture against a correctly generated binding produced an assertion that could never
//! be true. Neither shows up under tsc -- wasm-bindgen types a `JsValue` getter `any` -- so the
//! only place they surface is a red generated suite with no explanation in it.
//!
//! Split out of `assertions.rs`, which is over the 1,000-line cap and may not grow.

use super::render_assertion;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn variant(name: &str, fields: Vec<FieldDef>, serde_rename: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        serde_rename: serde_rename.map(str::to_string),
        ..EnumVariant::default()
    }
}

fn payload_field() -> FieldDef {
    FieldDef {
        name: "_0".to_string(),
        ty: TypeRef::String,
        ..FieldDef::default()
    }
}

/// `Payload { Unit, Custom(String) }` — data-carrying, serde's default (external) representation.
/// `Format { #[serde(rename = "md")] Markdown, Html }` — all unit variants, one renamed.
fn enums() -> Vec<EnumDef> {
    vec![
        EnumDef {
            name: "Payload".to_string(),
            variants: vec![
                variant("Unit", vec![], None),
                variant("Custom", vec![payload_field()], None),
            ],
            ..EnumDef::default()
        },
        EnumDef {
            name: "Format".to_string(),
            variants: vec![variant("Markdown", vec![], Some("md")), variant("Html", vec![], None)],
            ..EnumDef::default()
        },
    ]
}

fn type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Report".to_string(),
        fields: vec![
            FieldDef {
                name: "kind".to_string(),
                ty: TypeRef::Named("Payload".to_string()),
                ..FieldDef::default()
            },
            FieldDef {
                name: "format".to_string(),
                ty: TypeRef::Named("Format".to_string()),
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    }]
}

fn resolver() -> FieldResolver {
    let defs = type_defs();
    let enum_defs = enums();
    let result_fields: HashSet<String> = ["kind".to_string(), "format".to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&defs, &enum_defs),
        Some("Report".to_string()),
    )
    .with_wasm_enum_representations(&enum_defs)
}

/// The hand-written `result_enum_fields` config that admits a field to the wasm enum path at all.
/// Only presence is read; the class name is not referenced in the emitted code. ~keep
fn enum_field_config() -> HashMap<String, String> {
    HashMap::from([
        ("kind".to_string(), "Payload".to_string()),
        ("format".to_string(), "Format".to_string()),
    ])
}

fn render_wasm(field: &str, expected: &str) -> String {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(expected.to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver(),
        false,
        &enum_field_config(),
        "wasm",
        false,
        false,
        false,
    );
    out
}

/// GAP 2. The field is a `JsValue` carrying serde's external form, so a data variant arrives as
/// `{ Custom: "x" }`. The pre-fix lowering was `expect(result.kind).toBe("Custom")`, an object
/// compared against a string by `toBe` — false for every possible value. Executed under node the
/// old text fails on `{ Custom: "x" }` and the new one passes. ~keep
#[test]
fn a_data_carrying_enum_field_reads_the_variant_off_the_wire_objects_only_key() {
    let out = render_wasm("kind", "Custom");
    assert_eq!(
        out,
        "    expect((typeof result.kind === \"string\" ? result.kind : Object.keys(result.kind ?? {})[0])).toBe(\"Custom\");\n",
        "got: {out}"
    );
}

fn render_representation(enum_def: EnumDef) -> String {
    let defs = vec![TypeDef {
        name: "Report".to_string(),
        fields: vec![FieldDef {
            name: "kind".to_string(),
            ty: TypeRef::Named(enum_def.name.clone()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let result_fields = HashSet::from(["kind".to_string()]);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&defs, std::slice::from_ref(&enum_def)),
        Some("Report".to_string()),
    )
    .with_wasm_enum_representations(std::slice::from_ref(&enum_def));
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("kind".to_string()),
        value: Some(serde_json::Value::String("Custom".to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        &HashMap::from([("kind".to_string(), enum_def.name)]),
        "wasm",
        false,
        false,
        false,
    );
    out
}

#[test]
fn an_internally_tagged_enum_reads_the_declared_discriminator() {
    let out = render_representation(EnumDef {
        name: "Payload".to_string(),
        variants: vec![variant("Custom", vec![payload_field()], None)],
        serde_tag: Some("kind-name".to_string()),
        ..EnumDef::default()
    });
    assert_eq!(out, "    expect(result.kind?.[\"kind-name\"]).toBe(\"Custom\");\n");
}

#[test]
fn an_adjacently_tagged_enum_reads_the_declared_discriminator() {
    let out = render_representation(EnumDef {
        name: "Payload".to_string(),
        variants: vec![variant("Custom", vec![payload_field()], None)],
        serde_tag: Some("tag".to_string()),
        serde_content: Some("payload".to_string()),
        ..EnumDef::default()
    });
    assert_eq!(out, "    expect(result.kind?.[\"tag\"]).toBe(\"Custom\");\n");
}

#[test]
fn an_untagged_enum_emits_a_visible_skip_instead_of_inventing_a_variant_key() {
    let out = render_representation(EnumDef {
        name: "Payload".to_string(),
        variants: vec![variant("Custom", vec![payload_field()], None)],
        serde_untagged: true,
        ..EnumDef::default()
    });
    assert_eq!(
        out,
        "    // skipped: wasm untagged enum field 'kind' has no variant discriminator on the wire\n"
    );
    assert!(
        !out.contains("Object.keys"),
        "untagged payload was misread as an external tag: {out}"
    );
}

#[test]
fn the_internal_tag_assertion_executes_and_the_external_key_strategy_does_not() {
    let generated = render_representation(EnumDef {
        name: "Payload".to_string(),
        variants: vec![variant("Custom", vec![payload_field()], None)],
        serde_tag: Some("kind".to_string()),
        ..EnumDef::default()
    });
    let assertion = generated
        .trim()
        .replace("expect(", "assert.equal(")
        .replace(").toBe(", ", ");
    let source = format!(
        "import assert from 'node:assert/strict';\nconst result = {{ kind: {{ kind: 'Custom', value: 'x' }} }};\n{assertion}\nassert.notEqual(Object.keys(result.kind)[0], 'Custom');\n"
    );
    let output = std::process::Command::new("node")
        .arg("--input-type=module")
        .arg("--eval")
        .arg(source)
        .output()
        .expect("node is required for the enum representation oracle");
    assert!(
        output.status.success(),
        "node rejected the representation-aware assertion: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The SAME enum's unit variant is a bare string on the wire, so the accessor has to handle both
/// without indexing a string. This pins the `typeof` guard rather than assuming it. ~keep
#[test]
fn the_data_carrying_accessor_still_reads_a_bare_string_unit_variant() {
    let out = render_wasm("kind", "Unit");
    assert!(
        out.contains("typeof result.kind === \"string\" ? result.kind"),
        "got: {out}"
    );
    assert!(out.ends_with(".toBe(\"Unit\");\n"), "got: {out}");
}

/// GAP 3. `to_api_str()` returns the serde WIRE value, so a fixture naming the variant by its
/// Rust identifier was compared against a string the binding never produces. The pre-fix
/// lowering emitted `.toBe("Markdown")` against a getter yielding `"md"` — a permanently red
/// assertion on a correct binding. ~keep
#[test]
fn a_renamed_unit_variant_is_compared_against_its_wire_value_not_its_identifier() {
    let out = render_wasm("format", "Markdown");
    assert_eq!(out, "    expect(result.format).toBe(\"md\");\n", "got: {out}");
}

/// A fixture that already wrote the wire value is left alone — both spellings are accepted, which
/// is why the translation is a lookup with a passthrough rather than a rewrite. ~keep
#[test]
fn a_fixture_that_already_names_the_wire_value_is_untouched() {
    let out = render_wasm("format", "md");
    assert_eq!(out, "    expect(result.format).toBe(\"md\");\n", "got: {out}");
}

/// Renders the `equals` wasm enum path (the `render_wasm_enum_assertion` arm at
/// `assertions.rs`) against a `Format` enum whose sole variant is renamed to `wire` — the wire
/// value is a `#[serde(rename)]` string a consumer controls, not `render_wasm`'s fixed fixture.
fn render_wasm_with_wire(wire: &str) -> String {
    let type_defs = vec![TypeDef {
        name: "Report".to_string(),
        fields: vec![FieldDef {
            name: "format".to_string(),
            ty: TypeRef::Named("Format".to_string()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let enum_defs = vec![EnumDef {
        name: "Format".to_string(),
        variants: vec![variant("Markdown", vec![], Some(wire))],
        ..EnumDef::default()
    }];
    let result_fields: HashSet<String> = ["format".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&type_defs, &enum_defs),
        Some("Report".to_string()),
    );
    let config = HashMap::from([("format".to_string(), "Format".to_string())]);
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("format".to_string()),
        value: Some(serde_json::Value::String("Markdown".to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out, &assertion, "result", &resolver, false, &config, "wasm", false, false, false,
    );
    out
}

/// GAP 4 (security-relevant). The pre-fix wire comparison was `format!("... .toBe(\"{wire}\");\n")`
/// — `wire` spliced raw between hand-written double quotes with no escaping. A `#[serde(rename)]`
/// containing a `"` broke the emitted string literal outright (closed it early and left trailing
/// garbage tsc could not parse). Routed through `json_to_js`, the quote is escaped rather than
/// terminating the literal. See `multiword_wire_tag_tsc_tests.rs` for the real `tsc`/`node` proof
/// over the same shape. ~keep
#[test]
fn a_wire_value_containing_a_double_quote_is_escaped_not_spliced() {
    let out = render_wasm_with_wire("bad\"value");
    assert_eq!(out, "    expect(result.format).toBe(\"bad\\\"value\");\n", "got: {out}");
}

/// GAP 4 continued: an unescaped backslash before a recognized escape letter (`\t`, `\n`, ...)
/// silently changes MEANING rather than breaking syntax — `"C:\temp"` pre-fix decoded to
/// `C:<TAB>emp`, not the intended `C:\temp`, a false pass or false fail no `tsc` diagnostic would
/// ever catch. `json_to_js` doubles the backslash so it round-trips as data. ~keep
#[test]
fn a_wire_value_containing_a_backslash_is_escaped_not_spliced() {
    let out = render_wasm_with_wire("C:\\temp");
    assert_eq!(out, "    expect(result.format).toBe(\"C:\\\\temp\");\n", "got: {out}");
}

/// GAP 4 continued: a literal newline byte spliced raw into a hand-written double-quoted JS string
/// literal is an unterminated string literal — a deterministic `tsc` syntax error. `json_to_js`
/// emits the `\n` escape sequence instead of the raw byte. ~keep
#[test]
fn a_wire_value_containing_a_newline_is_escaped_not_spliced() {
    let out = render_wasm_with_wire("line1\nline2");
    assert_eq!(
        out, "    expect(result.format).toBe(\"line1\\nline2\");\n",
        "got: {out}"
    );
}

/// OVER-APPLICATION CONTROL: an unrenamed unit variant keeps its own spelling, and an all-unit
/// enum is never indexed. This is the shape that was already correct. ~keep
#[test]
fn an_unrenamed_unit_variant_keeps_the_plain_scalar_comparison() {
    let out = render_wasm("format", "Html");
    assert_eq!(out, "    expect(result.format).toBe(\"Html\");\n", "got: {out}");
    assert!(!out.contains("Object.keys"), "an all-unit enum was indexed: {out}");
}

/// OVER-APPLICATION CONTROL: `result_enum_fields` is hand-written config, so a field it names
/// that the IR cannot resolve has no data-carrying answer. `unwrap_or(false)` must leave such a
/// field on the comparison it already had rather than inventing an accessor for it. ~keep
#[test]
fn a_field_the_ir_cannot_resolve_keeps_the_plain_scalar_comparison() {
    let config = HashMap::from([("kind".to_string(), "Payload".to_string())]);
    let result_fields: HashSet<String> = ["kind".to_string()].into_iter().collect();
    let unanchored = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("kind".to_string()),
        value: Some(serde_json::Value::String("Custom".to_string())),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &unanchored,
        false,
        &config,
        "wasm",
        false,
        false,
        false,
    );
    assert_eq!(out, "    expect(result.kind).toBe(\"Custom\");\n", "got: {out}");
}

/// The non-`equals` arm is untouched by either fix; pinned so a later edit cannot quietly route
/// it through the new accessor. ~keep
#[test]
fn not_empty_on_an_enum_field_still_asserts_presence_only() {
    let assertion = Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("kind".to_string()),
        ..Default::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver(),
        false,
        &enum_field_config(),
        "wasm",
        false,
        false,
        false,
    );
    assert_eq!(out, "    expect(result.kind).toBeDefined();\n", "got: {out}");
}
