//! `node` and `wasm` are one TypeScript surface but two bindings, and a snippet must call the
//! binding it is generated for.
//!
//! A trailing argument the fixture leaves out is droppable only when the *target* declares that
//! parameter optional. The NAPI `.d.ts` writer widens a parameter whose type derives `Default` to
//! `settings?: …` (`napi::gen_bindings::errors::param_is_optional`), so a node call may end early.
//! wasm-bindgen has no such widening: it emits each parameter from the Rust signature alone, so
//! the parameter stays required and a call that stops before it is `TS2554: Expected 2 arguments,
//! but got 1` under the strict TypeScript the snippet validator compiles with.
//!
//! Kept out of `snippet.rs` and `tests.rs` (both remediation targets) because it owns a single
//! question with its own fixture, matching `wasm_optional_chain_tests.rs` next door. ~keep

use super::snippet::{SnippetContext, render_snippet_body};
use super::tests::{make_field, make_type};
use crate::core::ir::{FunctionDef, ParamDef, TypeDef, TypeRef};
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::Fixture;

/// `settings_has_default` is the NAPI widening trigger; the other two types never derive
/// `Default`, so nothing else in the fixture can move either target's arity.
fn batch_types(settings_has_default: bool) -> Vec<TypeDef> {
    let mut settings = make_type("SampleSettings", vec![make_field("mode", TypeRef::String)]);
    settings.has_default = settings_has_default;
    let mut item = make_type("SampleItem", vec![make_field("text", TypeRef::String)]);
    item.has_default = false;
    let mut report = make_type("SampleReport", vec![make_field("summary", TypeRef::String)]);
    report.has_default = false;
    vec![settings, item, report]
}

/// `settings_is_option` is the Rust-signature trigger, the only one wasm-bindgen honours.
fn batch_functions(settings_is_option: bool) -> Vec<FunctionDef> {
    let settings_type = TypeRef::Named("SampleSettings".into());
    vec![FunctionDef {
        name: "process_batch".into(),
        rust_path: "sample::process_batch".into(),
        params: vec![
            ParamDef {
                name: "items".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("SampleItem".into()))),
                ..ParamDef::default()
            },
            ParamDef {
                name: "settings".into(),
                ty: if settings_is_option {
                    TypeRef::Optional(Box::new(settings_type))
                } else {
                    settings_type
                },
                optional: settings_is_option,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("SampleReport".into()),
        error_type: Some("SampleError".into()),
        ..FunctionDef::default()
    }]
}

fn arg(name: &str, field: &str, element_type: &str, optional: bool) -> ArgMapping {
    ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "json_object".into(),
        optional,
        owned: false,
        element_type: Some(element_type.into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// The fixture supplies `items` and leaves `settings` out, which is what makes the omission
/// decision reachable at all.
fn snippet_for(lang: &str, settings_has_default: bool, settings_is_option: bool) -> String {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "process_batch".into();
    e2e_config.call.module = "@example/library".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.args = vec![
        arg("items", "input.items", "SampleItem", false),
        arg("settings", "input.settings", "SampleSettings", true),
    ];
    let fixture = Fixture {
        id: "process_batch".into(),
        description: "Process a batch".into(),
        input: serde_json::json!({"items": [{"text": "hello"}]}),
        ..Fixture::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    render_snippet_body(SnippetContext {
        lang,
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e_config,
        type_defs: &batch_types(settings_has_default),
        enums: &[],
        functions: &batch_functions(settings_is_option),
        wasm_type_prefix: "",
        config: &config,
    })
}

/// How many arguments the rendered snippet passes to the call under test.
fn argument_count(body: &str) -> usize {
    let line = body
        .lines()
        .find(|line| line.contains("processBatch("))
        .unwrap_or_else(|| panic!("snippet must call the function:\n{body}"));
    let start = line.find("processBatch(").expect("call present") + "processBatch(".len();
    let rest = &line[start..];
    let end = rest.rfind(')').expect("call is closed");
    let arguments = &rest[..end];
    if arguments.trim().is_empty() {
        return 0;
    }
    let mut depth = 0usize;
    let mut count = 1usize;
    for character in arguments.chars() {
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

/// The arity each target's own binding declares, asked of the same predicate that binding's
/// emitter uses, so this expectation cannot drift from the `.d.ts` the snippet is compiled
/// against. ~keep
fn required_argument_count(lang: &str, settings_has_default: bool, settings_is_option: bool) -> usize {
    let type_defs = batch_types(settings_has_default);
    let rule = crate::e2e::codegen::call_ir::ParamOptionalityRule::for_language(lang);
    batch_functions(settings_is_option)[0]
        .params
        .iter()
        .filter(|param| !rule.is_optional(param, &type_defs))
        .count()
}

/// The defect: a `Default`-deriving settings type is optional in node's `.d.ts` and required in
/// wasm's, and the snippet used to emit node's arity for both.
#[test]
fn each_target_is_called_with_the_arity_its_own_binding_declares() {
    for lang in ["node", "wasm"] {
        let body = snippet_for(lang, true, false);
        assert_eq!(
            argument_count(&body),
            required_argument_count(lang, true, false),
            "the {lang} snippet must pass every argument the {lang} binding declares required:\n{body}"
        );
    }
    assert_eq!(
        required_argument_count("node", true, false),
        1,
        "a Default-deriving parameter type is what NAPI widens to `settings?:`"
    );
    assert_eq!(
        required_argument_count("wasm", true, false),
        2,
        "wasm-bindgen widens nothing, so the same parameter stays required"
    );
}

/// Negative control for the widening trigger: with the settings type no longer deriving `Default`,
/// node declares the parameter required too, so node must spell it as well. A fix that special-cased
/// `lang == "wasm"` would leave this snippet a `TS2554` against its own `.d.ts`.
#[test]
fn node_also_fills_a_parameter_its_binding_declares_required() {
    let body = snippet_for("node", false, false);
    assert_eq!(required_argument_count("node", false, false), 2);
    assert_eq!(
        argument_count(&body),
        2,
        "without the Default widening, node's own declaration requires the argument:\n{body}"
    );
}

/// Negative control for the omission itself: a genuinely `Option<T>` parameter is optional in both
/// bindings, so neither target may pad the call with an `undefined` a reader has to ignore.
#[test]
fn neither_target_pads_a_parameter_both_bindings_declare_optional() {
    for lang in ["node", "wasm"] {
        let body = snippet_for(lang, false, true);
        assert_eq!(required_argument_count(lang, false, true), 1);
        assert_eq!(
            argument_count(&body),
            1,
            "an `Option<T>` parameter is optional in every JS binding, so {lang} must end the call \
             early rather than pass `undefined`:\n{body}"
        );
    }
}
