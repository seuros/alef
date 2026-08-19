//! The C generated-test-file path must resolve a call's result type from the core IR.
//!
//! `E2eCodegen::generate` gained a `functions` parameter so `render_test_file` could stop
//! passing an empty IR slice. Before that, three things on this path were unreachable —
//! `resolve_ir_result_type`, `resolve_raw_c_result_type`, and argument `element_type`
//! inference — and `fallback_result_type_name` fired for every fixture, naming the result
//! type by PascalCasing the call name (`complete` → `Complete`).
//!
//! The wrong name is not the real damage. `result_type_name` feeds `parent_is_ir_type`, and
//! `ensure_leaf_field_exists` returns `Ok(())` immediately when that is false — so a
//! fabricated result type does not fail generation, it *switches off* the nested-field
//! verification for that fixture. These tests pin both halves: that resolution happens, and
//! that verification is consequently live on this path.

use alef::core::config::NewAlefConfig;
use alef::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::c::CCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// `complete` is a free `pub fn complete(prompt: String) -> Result<CompletionResponse, String>`.
/// The call deliberately carries no `[overrides.c] result_type`: pinning one is the workaround
/// this change removes, and pinning it here would make every assertion below vacuous.
const CONFIG_TOML: &str = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "gatelib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
result_fields = ["content", "ghost"]

[crates.e2e.call]
function = "complete"
module = "gatelib"
result_var = "result"
args = [
  { name = "prompt", field = "input.prompt", type = "string" },
]

[crates.e2e.call.overrides.c]
header = "gatelib.h"
function = "sample_complete"
prefix = "sample"
"#;

/// The IR the extractor produces for that crate: one free function, one return type.
fn ir_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "complete".to_string(),
        rust_path: "gatelib::complete".to_string(),
        return_type: TypeRef::Named("CompletionResponse".to_string()),
        error_type: Some("String".to_string()),
        ..FunctionDef::default()
    }]
}

/// `CompletionResponse` has a `content` field and nothing else. The absence of `ghost` is
/// what the nested-field check is supposed to catch.
fn ir_types() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "CompletionResponse".to_string(),
        rust_path: "gatelib::CompletionResponse".to_string(),
        fields: vec![FieldDef {
            name: "content".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }]
}

fn group_asserting_field(field: &str) -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "completion".to_string(),
        fixtures: vec![Fixture {
            id: "complete_basic".to_string(),
            category: Some("completion".to_string()),
            description: "basic completion".to_string(),
            input: serde_json::json!({ "prompt": "hello" }),
            assertions: vec![Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some(field.to_string()),
                ..Assertion::default()
            }],
            source: "fixtures/complete_basic.json".to_string(),
            ..Fixture::default()
        }],
    }]
}

fn generate(groups: &[FixtureGroup], functions: &[FunctionDef], type_defs: &[TypeDef]) -> anyhow::Result<String> {
    let config: NewAlefConfig = toml::from_str(CONFIG_TOML).expect("config parses");
    let resolved = config.clone().resolve().expect("config resolves").remove(0);
    let e2e = config.crates[0].e2e.clone().expect("e2e config present");
    let files = CCodegen.generate(groups, &e2e, &resolved, type_defs, &[], functions, &[])?;
    let test_file = files
        .iter()
        .find(|file| {
            file.path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("test_") && name.ends_with(".c"))
        })
        .expect("a per-category test file must be emitted");
    Ok(test_file.content.clone())
}

/// The decisive one. With the IR threaded into `generate`, the accessor the generated test
/// calls must be named from the declared return type — `CompletionResponse` — and never from
/// the PascalCased call name. The two accessor spellings differ by exactly this fix.
#[test]
fn should_name_accessors_from_the_ir_return_type_on_the_test_file_path() {
    let content = generate(&group_asserting_field("content"), &ir_functions(), &ir_types())
        .expect("generation succeeds for a field the IR type really has");

    assert!(
        content.contains("sample_completion_response_content(result)"),
        "the accessor must be derived from `Result<CompletionResponse, _>`'s Ok type, but the \
         generated test reads:\n{content}"
    );
    assert!(
        !content.contains("sample_complete_content("),
        "`Complete` is the PascalCased call name, not a type; seeing its accessor means the \
         test-file path is still resolving nothing:\n{content}"
    );
}

/// Control: the fallback has to stay, because a call the IR genuinely cannot answer for still
/// needs *some* result type. With no IR at all the generator must keep producing the
/// documented PascalCase name rather than failing.
#[test]
fn should_still_fall_back_to_the_call_name_when_the_ir_cannot_answer() {
    let content = generate(&group_asserting_field("content"), &[], &[])
        .expect("generation succeeds with no IR, exactly as before");

    assert!(
        content.contains("sample_complete_content(result)"),
        "with no IR to consult, the PascalCased call name is the only answer available and the \
         fallback must still produce it:\n{content}"
    );
}

/// The real payload. `ensure_leaf_field_exists` only runs when `parent_is_ir_type` is true,
/// and that flag is `type_defs.iter().any(|t| t.name == result_type_name)` — so it was dead
/// on this path for as long as `result_type_name` was a fabricated name. `ghost` is not a
/// field of `CompletionResponse`, and generation must now fail rather than emit
/// `sample_completion_response_ghost()`, a C symbol no binding generates.
#[test]
fn should_reject_a_leaf_field_the_ir_type_does_not_have() {
    let error = generate(&group_asserting_field("ghost"), &ir_functions(), &ir_types())
        .expect_err("a field absent from the resolved IR type must fail generation");
    let message = format!("{error:#}");

    assert!(
        message.contains("CompletionResponse"),
        "the diagnostic must name the IR type the walk was standing on: {message}"
    );
    assert!(
        message.contains("ghost"),
        "the diagnostic must name the offending field: {message}"
    );
}

/// The negative control that keeps the previous test from passing for the wrong reason.
///
/// It used to assert the defect directly: identical fixture, identical `type_defs`, only the IR
/// functions withheld, and generation *succeeded*, emitting `sample_complete_ghost(result)` — an
/// accessor for a field that does not exist, against a fabricated `Complete` that matched no
/// entry in `type_defs`, which is exactly what switched `ensure_leaf_field_exists` off.
///
/// That path now refuses instead of inventing a name, so the old assertion cannot stand. But the
/// control still has a job, and it is a bigger one than before: with both cases failing, the
/// sibling test would pass unchanged if generation had simply started refusing *everything*. So
/// this pins that the two failures are different failures — this one names the unresolvable call
/// and the config knob that fixes it, and says nothing about the leaf field, which is what proves
/// the sibling's `ghost`/`CompletionResponse` diagnostic really came from the leaf-field walk. ~keep
#[test]
fn should_refuse_rather_than_default_allow_when_the_result_type_did_not_resolve() {
    let error = generate(&group_asserting_field("ghost"), &[], &ir_types())
        .expect_err("an unresolvable result type must fail generation rather than default-allow the leaf check");
    let message = format!("{error:#}");

    assert!(
        message.contains("complete"),
        "the diagnostic must name the call that could not be resolved: {message}"
    );
    assert!(
        message.contains("result_type"),
        "the diagnostic must name the config knob that fixes it: {message}"
    );
    assert!(
        !message.contains("CompletionResponse") && !message.contains("ghost"),
        "this failure must NOT be the leaf-field failure — if it were, the sibling test would be \
         passing on a blanket refusal rather than on the field walk: {message}"
    );
}
