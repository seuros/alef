//! Regression coverage for `exclusions::function_binding_excluded_for_language`.
//!
//! Split out of [`coverage`] rather than added there: `coverage.rs` was already near this
//! crate's per-file line cap when this module was written, and these tests share nothing with
//! that file beyond the `super::*` fixture helpers every sibling test module already reuses.
//!
//! Sibling of `coverage::excluded_function_drops_only_the_excluding_languages_cell_from_expected`
//! and modeled on the same shape: a fixture whose call resolves to a `binding_excluded` IR
//! function must drop out of `expected` (and therefore `missing`) for a non-Rust language, while
//! the "rust" carve-out and an ordinary, non-excluded function are unaffected.

use super::*;

/// The bug this guards against: `#[alef::skip]`/`#[doc(hidden)]` sets `FunctionDef`'s IR-level
/// `binding_excluded` flag, which `function_excluded_for_language` never consults (it only
/// reads `alef.toml`-configured `exclude_functions` lists). Before
/// `function_binding_excluded_for_language` existed, a function a Rust author explicitly opted
/// out of every binding still entered `coverage.expected` for every non-Rust language, and the
/// ledger reported the resulting absence as an unsilenceable coverage gap even though the
/// function was never meant to be bindable at all. The "rust" cell must stay expected, matching
/// `docs::language_pages::mod::generate_lang_doc`'s `lang == Language::Rust ||
/// !binding_excluded` carve-out: the function still exists in Rust source, so Rust's own page
/// (and this generator's own recipe) keeps documenting it.
#[test]
fn binding_excluded_function_drops_the_non_rust_cell_but_keeps_rust() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "excluded_core_fn".into();
    let functions = vec![crate::core::ir::FunctionDef {
        name: "excluded_core_fn".into(),
        binding_excluded: true,
        ..crate::core::ir::FunctionDef::default()
    }];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &functions,
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(
        &[fixture],
        &["python".into(), "rust".into()],
        &snippet_config,
        &context,
        &[],
    )
    .expect("a binding_excluded function must not abort the run");

    let python_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "python".into(),
    };
    let rust_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "rust".into(),
    };
    assert_eq!(
        report.coverage.expected,
        vec![rust_key.clone()],
        "a `binding_excluded` function must drop the python cell from `expected` while the \
         rust carve-out keeps the rust cell: {:?}",
        report.coverage.expected
    );
    assert!(
        !report.coverage.expected.contains(&python_key),
        "a `binding_excluded` function must not be expected for python: {:?}",
        report.coverage.expected
    );
    assert!(
        report.coverage.missing.is_empty(),
        "a `binding_excluded` cell is not a coverage gap -- it must never have been expected \
         in the first place, so it must not appear in `missing` either: {:?}",
        report.coverage.missing
    );
    assert!(
        report.coverage.generated.contains(&rust_key),
        "the rust carve-out cell must still render: {:?}",
        report.coverage.generated
    );
}

/// The peer's positive control: an ordinary, `binding_excluded: false` function must still
/// enter `expected` (and `generated`) once the IR context carries real `FunctionDef`s -- proof
/// that `function_binding_excluded_for_language` reports an exclusion only for the flagged
/// function, not for every cell once a populated `functions` slice is in play.
#[test]
fn non_excluded_function_still_enters_expected_and_generated() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "ordinary_core_fn".into();
    let functions = vec![crate::core::ir::FunctionDef {
        name: "ordinary_core_fn".into(),
        binding_excluded: false,
        ..crate::core::ir::FunctionDef::default()
    }];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &functions,
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["python".into()], &snippet_config, &context, &[])
            .expect("an ordinary function must not abort the run");

    let python_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "python".into(),
    };
    assert_eq!(
        report.coverage.expected,
        vec![python_key.clone()],
        "a normal function must still be expected: {:?}",
        report.coverage.expected
    );
    assert_eq!(
        report.coverage.generated,
        vec![python_key],
        "a normal function must still generate: {:?}",
        report.coverage.generated
    );
}
