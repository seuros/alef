//! Regression coverage for the adapter/attribute distinction in
//! `exclusions::function_binding_excluded_for_language`.
//!
//! `mark_adapter_handled_methods` (`src/cli/pipeline/extract/services.rs`) sets
//! `MethodDef::binding_excluded = true` on every method an `[[crates.adapters]]` entry names,
//! with no per-language distinction: it exists only to keep the *generic* method codegen path
//! from double-emitting a method a backend's own adapter machinery already handles. Before this
//! module's fix, `function_binding_excluded_for_language` treated that flag exactly like a
//! genuine `#[alef::skip]`/`#[doc(hidden)]` exclusion and dropped the cell from `expected` for
//! every language -- even languages where the method is fully bound, either through the adapter
//! itself or through the ordinary per-method codegen loop (which does not filter on
//! `MethodDef::binding_excluded` at all). Sibling of `binding_excluded.rs`, which pins the
//! genuine-exclusion side of this same predicate.

use super::*;
use crate::core::config::{AdapterConfig, AdapterPattern};
use crate::core::ir::MethodDef;

fn adapter_config(owner_type: &str, core_path: &str, skip_languages: Vec<String>) -> AdapterConfig {
    AdapterConfig {
        name: core_path.to_string(),
        pattern: AdapterPattern::AsyncMethod,
        core_path: core_path.to_string(),
        params: Vec::new(),
        returns: None,
        error_type: None,
        owner_type: Some(owner_type.to_string()),
        item_type: None,
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages,
    }
}

/// The bug: a method that is `binding_excluded` purely because `[[crates.adapters]]` names it
/// (reason text mirrors `mark_adapter_handled_methods`'s "handled by [[crates.adapters]] entry")
/// must still enter `coverage.expected` -- and still render -- for a language the adapter does
/// not name in `skip_languages`. This is the decisive case from the liter-llm regression:
/// `DefaultClient::chat` is `binding_excluded` in the IR but fully present in every generated
/// binding because its `async_method` adapter has an empty `skip_languages`.
#[test]
fn adapter_handled_method_still_enters_expected_and_generated_for_an_unskipped_language() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    let type_defs = [TypeDef {
        name: "DefaultClient".into(),
        methods: vec![MethodDef {
            name: "chat".into(),
            binding_excluded: true,
            binding_exclusion_reason: Some("handled by [[crates.adapters]] entry `chat`".into()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    }];
    let crate_config = ResolvedCrateConfig {
        adapters: vec![adapter_config("DefaultClient", "chat", Vec::new())],
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &type_defs,
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["python".into()], &snippet_config, &context, &[])
            .expect("an adapter-handled method must not abort the run");

    let python_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "python".into(),
    };
    assert_eq!(
        report.coverage.expected,
        vec![python_key.clone()],
        "an adapter-handled method with no skip_languages entry for python must still be \
         expected: {:?}",
        report.coverage.expected
    );
    assert_eq!(
        report.coverage.generated,
        vec![python_key],
        "an adapter-handled method with no skip_languages entry for python must still render: \
         {:?}",
        report.coverage.generated
    );
    assert!(
        report.coverage.missing.is_empty(),
        "an adapter-handled, still-bound method must not be reported as a coverage gap: {:?}",
        report.coverage.missing
    );
}

/// The adapter's own `skip_languages` is the one place this pattern legitimately says "no
/// binding surface here": when the adapter that would otherwise cover a language explicitly
/// excludes it, the generic codegen path is still suppressed (that is what
/// `mark_adapter_handled_methods` is for) and no adapter substitutes for it either. That cell
/// must behave like a genuine exclusion -- absent from `expected`, absent from `missing`.
#[test]
fn adapter_handled_method_stays_excluded_for_a_skipped_language() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    let type_defs = [TypeDef {
        name: "DefaultClient".into(),
        methods: vec![MethodDef {
            name: "chat".into(),
            binding_excluded: true,
            binding_exclusion_reason: Some("handled by [[crates.adapters]] entry `chat`".into()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    }];
    let crate_config = ResolvedCrateConfig {
        adapters: vec![adapter_config("DefaultClient", "chat", vec!["python".into()])],
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &type_defs,
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["python".into()], &snippet_config, &context, &[])
            .expect("a skip_languages-excluded method must not abort the run");

    assert!(
        report.coverage.expected.is_empty(),
        "python is named in the adapter's skip_languages, so the cell must not be expected: {:?}",
        report.coverage.expected
    );
    assert!(
        report.coverage.missing.is_empty(),
        "a legitimately excluded cell must never have been expected, so it is not missing \
         either: {:?}",
        report.coverage.missing
    );
}

/// The peer's control: a method `binding_excluded` because of a genuine source attribute (no
/// matching `[[crates.adapters]]` entry at all) must keep behaving exactly like
/// `binding_excluded.rs`'s free-function case -- dropped from `expected`, absent from `missing`.
/// This is what proves the fix distinguishes the two causes instead of just returning `false`
/// unconditionally for every `binding_excluded` method.
#[test]
fn attribute_excluded_method_with_no_adapter_stays_excluded() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "internal_only".into();
    let type_defs = [TypeDef {
        name: "DefaultClient".into(),
        methods: vec![MethodDef {
            name: "internal_only".into(),
            binding_excluded: true,
            binding_exclusion_reason: Some("alef(skip)".into()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    }];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &type_defs,
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["python".into()], &snippet_config, &context, &[])
            .expect("an attribute-excluded method must not abort the run");

    assert!(
        report.coverage.expected.is_empty(),
        "a genuinely `#[alef::skip]`'d method with no adapter entry must not be expected: {:?}",
        report.coverage.expected
    );
    assert!(
        report.coverage.missing.is_empty(),
        "a genuine exclusion must never have been expected, so it is not missing either: {:?}",
        report.coverage.missing
    );
}
