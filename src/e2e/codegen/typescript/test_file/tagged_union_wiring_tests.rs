//! Regression coverage for the authorities `render_test_case` hands its `FieldResolver`.
//!
//! ~keep This file exists because the defect was in the WIRING, not in the refusal. Testing
//! `field_refusal::refusal_line` against a hand-built resolver passes just as happily while
//! `render_test_case` builds its own resolver with an empty method-call set — which is exactly
//! what it did: `FieldResolver::new(.., &HashSet::new())`, where gleam, kotlin, dart, python,
//! elixir, rust, java and zig all pass `effective_fields_method_calls`. `tagged_union_split`
//! therefore answered `None` for every path and the node/wasm suites emitted a raw dotted
//! accessor across a tagged-union boundary, which does not compile: NAPI flattens a data enum
//! into one object with a discriminant plus optional sibling fields, so the variant segment has
//! no member (`TS2339`). These tests drive the real entry point so the wiring is what is tested.
//!
//! A `fields_method_calls` entry names `<enum field path>.<variant>` — `shape.circle` for the
//! crossing that `shape.circle.radius` walks — matching `kotlin/assertions/tests.rs`.

use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::Assertion;

fn not_empty_on(field: &str) -> Assertion {
    Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

/// Render one fixture through the production entry point, with `declared_crossings` standing in
/// for the consumer's `[e2e].fields_method_calls`.
fn render(lang: &str, declared_crossings: &[&str], assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: "describe_shape".to_string(),
        description: "Describe a shape".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "describeShape".to_string(),
            module: "sampleLib".to_string(),
            result_var: "result".to_string(),
            r#async: true,
            ..CallConfig::default()
        },
        fields_method_calls: declared_crossings.iter().map(|entry| (*entry).to_string()).collect(),
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig::default();
    let type_defs: Vec<TypeDef> = Vec::new();
    let enums: Vec<EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
    let mut referenced_enums = std::collections::BTreeSet::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        lang,
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &type_defs,
        &enums,
        &[],
        "",
        &config,
        &mut referenced_enums,
        &errors,
    );
    out
}

/// The defect itself: a fixture path the consumer declared as a tagged-union crossing must be
/// recorded as a skip, never spelled as `result.shape.circle.radius`.
#[test]
fn a_declared_union_crossing_renders_a_skip_not_a_dotted_accessor() {
    let out = render("node", &["shape.circle"], vec![not_empty_on("shape.circle.radius")]);

    assert!(
        out.contains(
            "// skipped: field 'shape.circle.radius' crosses a tagged-union variant boundary \
             (no variant member on the generated TypeScript type)"
        ),
        "the declared crossing must be routed through the FieldSkip funnel, got:\n{out}"
    );
    // The skip line quotes the fixture path, so this must name the ACCESSOR spelling — asserting
    // on the bare path would match the refusal itself and pass without proving anything. ~keep
    assert!(
        !out.contains("result.shape"),
        "no accessor may spell the variant segment, got:\n{out}"
    );
}

/// wasm is emitted by this same generator, off the same config, and wasm-bindgen's structural
/// `.d.ts` union is no more narrowable from a straight-line assertion than NAPI's flat object.
#[test]
fn the_wasm_language_gets_the_same_refusal_as_node() {
    let out = render("wasm", &["shape.circle"], vec![not_empty_on("shape.circle.radius")]);

    assert!(
        out.contains("crosses a tagged-union variant boundary"),
        "wasm shares the resolver wiring and must share the refusal, got:\n{out}"
    );
}

/// The control that makes "skip everything" fail: an ordinary struct field on the very same
/// fixture, with the very same crossing declared, must still render its real accessor.
#[test]
fn an_ordinary_struct_field_still_renders_its_normal_access() {
    let out = render(
        "node",
        &["shape.circle"],
        vec![not_empty_on("shape.circle.radius"), not_empty_on("summary.title")],
    );

    assert!(
        out.contains("result.summary.title"),
        "an ordinary field must keep its accessor, got:\n{out}"
    );
    assert!(
        !out.contains("// skipped: field 'summary.title'"),
        "the ordinary field must not be swept up by the union refusal, got:\n{out}"
    );
}

/// The refusal is keyed on the consumer's own declaration, not on the path's shape: with nothing
/// declared, the same deep path is spelled exactly as it was before the refusal existed.
#[test]
fn an_undeclared_deep_path_keeps_its_accessor() {
    let out = render("node", &[], vec![not_empty_on("shape.circle.radius")]);

    assert!(
        out.contains("result.shape.circle.radius"),
        "with no crossing declared nothing changes, got:\n{out}"
    );
}
