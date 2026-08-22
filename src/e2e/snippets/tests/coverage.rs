//! Coverage-ledger and per-language recipe tests for the snippet driver.
//!
//! Sibling half of [`super`]; see that module for the split's rationale. `use super::*` picks up
//! both the fixture helpers defined there and the driver items that module globs in.

use super::*;

#[test]
fn snippet_generator_resolution_rejects_unknown_languages() {
    let error = snippet_generators(&["unknown".into()])
        .err()
        .expect("unknown language must fail");
    assert_eq!(
        error.to_string(),
        "no e2e code generator registered for snippet language `unknown`"
    );
}

#[test]
fn snippet_generator_resolution_rejects_alias_duplicates() {
    let error = snippet_generators(&["rust".into(), "rust_core".into()])
        .err()
        .expect("duplicate generator selection must fail");
    assert_eq!(
        error.to_string(),
        "duplicate snippet language resolves to e2e code generator `rust`"
    );
}

#[test]
fn markdown_wrapper_uses_backend_body_and_metadata() {
    let docs = FixtureDocs {
        topic: "api".into(),
        stem: None,
        paths: BTreeMap::new(),
        title: Some("Example".into()),
        description: None,
        input: None,
        shows: Vec::new(),
        error: None,
        presentation: None,
        client: None,
        side_effects: SideEffectClass::Network,
        coverage_exceptions: BTreeMap::new(),
    };

    let rendered = render_snippet_markdown(
        "backend_call()",
        &documented_fixture(),
        &docs,
        "python",
        DocumentationLanguage::Binding(Language::Python),
    );

    assert!(rendered.starts_with("---\nid: fixture_python_extension_owned\nlanguage: python\ntarget: python\n"));
    assert!(rendered.contains("requires: []\nside_effect: network\n---"));
    assert!(rendered.ends_with("```python title=\"Python\"\nbackend_call()\n```\n"));
    assert!(!rendered.contains("Backend-owned body"));
    assert!(!rendered.contains("Example"));
}

#[test]
fn extension_owned_recipe_satisfies_expected_coverage() {
    let fixture = documented_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "built_in_would_fail".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
        body: "extension_call()",
    })];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &extensions)
            .expect("extension snippet report renders");

    assert_eq!(report.coverage.expected.len(), 1);
    assert_eq!(report.coverage.generated, report.coverage.expected);
    assert!(report.coverage.missing.is_empty());
    assert!(report.snippets[0].file.content.contains("extension_call()"));
}

#[test]
fn c_trait_bridge_vtable_recipe_counts_as_generated() {
    let mut fixture = documented_fixture();
    fixture.call = Some("register_sample_backend".into());
    fixture.args = vec![crate::core::config::e2e::ArgMapping {
        name: "backend".into(),
        field: "backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some("SampleBackend".into()),
    }];
    let mut e2e = E2eConfig::default();
    let mut call = crate::core::config::e2e::CallConfig::default();
    call.overrides.insert(
        "python".into(),
        crate::core::config::e2e::CallOverride {
            function: Some("register_sample_backend".into()),
            ..Default::default()
        },
    );
    e2e.calls.insert("register_sample_backend".into(), call);
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig {
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "SampleBackend".into(),
            register_fn: Some("register_sample_backend".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [TypeDef {
        name: "SampleBackend".into(),
        is_trait: true,
        ..TypeDef::default()
    }];
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &type_defs,
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(
        &[fixture],
        &["c".into(), "python".into()],
        &snippet_config,
        &context,
        &[],
    )
    .expect("unsupported C recipe belongs in the coverage ledger");

    assert_eq!(report.coverage.expected.len(), 2);
    assert_eq!(report.coverage.generated.len(), 2);
    assert!(report.coverage.missing.is_empty());
    let c = report
        .snippets
        .iter()
        .find(|snippet| snippet.language == "c")
        .expect("C trait bridge snippet");
    assert!(c.file.content.contains("register_sample_backend"));
    assert!(c.file.content.contains(".free_user_data = sample_free_context"));
}

#[test]
fn unclaimed_domain_fixture_is_recorded_as_missing() {
    let mut fixture = documented_fixture();
    fixture.asyncapi = Some(crate::e2e::fixture::AsyncApiFixture {
        spec: serde_json::json!({"asyncapi": "3.0.0"}),
        expected: serde_json::Value::Null,
        validation: None,
    });
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let e2e = E2eConfig::default();
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["go".into()], &snippet_config, &context, &[])
        .expect("unclaimed domain recipe belongs in coverage report");

    assert!(report.snippets.is_empty());
    assert_eq!(report.coverage.missing.len(), 1);
    assert_eq!(
        report.coverage.missing[0].reason,
        "AsyncAPI fixture requires an extension-owned documentation recipe"
    );
}

#[test]
fn empty_call_identity_is_missing_instead_of_generated() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let e2e = E2eConfig::default();
    assert!(e2e.call.function.is_empty());
    assert!(e2e.call.module.is_empty());
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(
        &[fixture],
        &["go".into(), "java".into()],
        &snippet_config,
        &context,
        &[],
    )
    .expect("missing call identities belong in the coverage ledger");

    assert!(report.snippets.is_empty());
    assert!(report.coverage.generated.is_empty());
    assert_eq!(report.coverage.expected.len(), 2);
    assert_eq!(report.coverage.missing.len(), 2);
    assert!(
        report
            .coverage
            .missing
            .iter()
            .all(|missing| missing.reason.contains("has no function identity"))
    );
}

#[test]
fn language_function_override_supplies_missing_default_identity() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.overrides.insert(
        "go".into(),
        crate::core::config::e2e::CallOverride {
            function: Some("process".into()),
            ..Default::default()
        },
    );
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["go".into()], &snippet_config, &context, &[])
        .expect("language override supplies a valid identity");

    assert_eq!(report.coverage.generated, report.coverage.expected);
    assert!(report.coverage.missing.is_empty());
    assert!(!report.snippets[0].file.content.contains("pkg.()"));
}

/// The peer's positive control: a fixture's function excluded via `[crates.wasm]
/// exclude_functions` must drop out of `expected` for wasm specifically, while the very
/// same fixture -- same call, same function identity -- stays expected (and generated)
/// for a language that does not exclude it. A version of this check that ignored the
/// exclusion entirely would still pass every other assertion in this file (the fixture
/// renders fine on both targets absent the exclusion) but would fail the two assertions
/// below, which is what makes this the load-bearing test rather than a truthiness check.
#[test]
fn excluded_function_drops_only_the_excluding_languages_cell_from_expected() {
    let fixture = documented_fixture();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "excluded_fn".into();
    let wasm_config = toml::from_str::<crate::core::config::WasmConfig>("exclude_functions = [\"excluded_fn\"]")
        .expect("wasm config with exclude_functions parses");
    let crate_config = ResolvedCrateConfig {
        wasm: Some(wasm_config),
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(
        &[fixture],
        &["wasm".into(), "go".into()],
        &snippet_config,
        &context,
        &[],
    )
    .expect("an excluded function must not abort the run");

    let wasm_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "wasm".into(),
    };
    let go_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "go".into(),
    };
    assert_eq!(
        report.coverage.expected,
        vec![go_key.clone()],
        "wasm's exclude_functions entry must remove the wasm cell from `expected` while \
         leaving go's untouched: {:?}",
        report.coverage.expected
    );
    assert!(
        !report.coverage.expected.contains(&wasm_key),
        "excluded cell must not be expected for wasm: {:?}",
        report.coverage.expected
    );
    assert_eq!(report.coverage.generated, vec![go_key]);
    assert!(
        report.coverage.missing.is_empty(),
        "an excluded cell is not a coverage gap -- it must never have been expected in the \
         first place, so it must not appear in `missing` either: {:?}",
        report.coverage.missing
    );
    assert_eq!(report.coverage.generated_paths.len(), 1);
    assert!(!report.coverage.generated_paths[0].starts_with("wasm"));
}

/// Sibling of `excluded_function_drops_only_the_excluding_languages_cell_from_expected`
/// for the visitor/trait-bridge convention: a fixture using [`Fixture::visitor`] must drop
/// out of `expected` for a language whose `exclude_functions` names the
/// [`crate::e2e::fixture::VISITOR_EXCLUDE_FUNCTION_NAME`] token, even though the fixture's
/// *call* resolves to an ordinary, non-excluded function -- `function_excluded_for_language`
/// alone cannot catch this, since it only inspects the call's function name. The same
/// fixture must stay expected (and generated) for a language with no such exclusion.
#[test]
fn visitor_fixture_excluded_by_visitor_token_drops_only_the_excluding_languages_cell() {
    let mut fixture = documented_fixture();
    fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
        callbacks: BTreeMap::new(),
    });
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    let kotlin_android_config =
        toml::from_str::<crate::core::config::KotlinAndroidConfig>("exclude_functions = [\"visitor\"]")
            .expect("kotlin_android config with exclude_functions parses");
    let crate_config = ResolvedCrateConfig {
        kotlin_android: Some(kotlin_android_config),
        // Go's visitor snippet renderer bails without a resolvable options type (see
        // `go::snippet::render_snippet_body`'s "needs an options type for its visitor"
        // guard) -- this is otherwise unrelated to the exclusion under test, so a bare
        // `options_type` is enough to let the go control cell render successfully.
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            options_type: Some("Options".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(
        &[fixture],
        &["kotlin_android".into(), "go".into()],
        &snippet_config,
        &context,
        &[],
    )
    .expect("a visitor-excluded language must not abort the run");

    let kotlin_android_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "kotlin_android".into(),
    };
    let go_key = SnippetCoverageKey {
        fixture_id: "extension_owned".into(),
        language: "go".into(),
    };
    assert_eq!(
        report.coverage.expected,
        vec![go_key.clone()],
        "kotlin_android's `exclude_functions = [\"visitor\"]` must remove the \
         kotlin_android cell from `expected` while leaving go's untouched: {:?}",
        report.coverage.expected
    );
    assert!(
        !report.coverage.expected.contains(&kotlin_android_key),
        "the visitor-excluded cell must not be expected for kotlin_android: {:?}",
        report.coverage.expected
    );
    assert_eq!(report.coverage.generated, vec![go_key]);
    assert!(
        report.coverage.missing.is_empty(),
        "a visitor-excluded cell is not a coverage gap -- it must never have been expected \
         in the first place: {:?}",
        report.coverage.missing
    );
}

#[test]
fn documentation_rendering_is_independent_of_test_harness_skips() {
    let mut fixture = documented_fixture();
    fixture.skip = Some(crate::e2e::fixture::SkipDirective {
        languages: vec!["ruby".into()],
        reason: Some("The test harness cannot exercise this protocol operation".into()),
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "built_in_would_fail".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
        body: "extension_call()",
    })];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["ruby".into()], &snippet_config, &context, &extensions)
            .expect("test harness skip does not suppress the extension-owned recipe");

    assert_eq!(report.coverage.generated, report.coverage.expected);
    assert!(report.coverage.missing.is_empty());
}

/// Regression test for the `c` plugin-api doc snippets that called a symbol
/// that does not exist (`{prefix}_clear_ocr_backends`, the pluralised `clear_fn`
/// config text, instead of the real singular `{prefix}_clear_ocr_backend` the FFI
/// backend derives from the trait name). Those fixtures are `skip.languages = ["c"]`
/// because the C API cannot expose a host-language callback, have no
/// extension-owned recipe, and no per-language call override — so the naive
/// `trait_bridge_function_identity` fallback must not run for them; the
/// pair should land in `coverage.missing`, not produce a broken snippet.
#[test]
fn skipped_fixture_without_extension_recipe_omits_c_snippet_and_records_missing() {
    let mut fixture = documented_fixture();
    fixture.call = Some("clear_ocr_backends".into());
    fixture.skip = Some(crate::e2e::fixture::SkipDirective {
        languages: vec!["c".into()],
        reason: Some("The C API does not expose the clear call that pairs with registration".into()),
    });
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".into(),
            clear_fn: Some("clear_ocr_backends".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &[])
        .expect("a skipped fixture with no recipe belongs in the coverage ledger, not an error");

    assert!(report.snippets.is_empty());
    assert!(report.coverage.generated.is_empty());
    assert_eq!(report.coverage.missing.len(), 1);
    assert_eq!(
        report.coverage.missing[0].reason,
        "built-in `c` snippet recipe has no function identity; configure a call function or provide an extension-owned documentation recipe"
    );
}

/// Companion to the regression test above: a fixture skipped for `c` but
/// backed by an extension-owned recipe must still render — doc rendering
/// stays independent of test-harness skips whenever a real recipe exists.
/// The extension loop in `render_snippet_body` runs before the skip check
/// this fix introduces, so this must keep passing unchanged.
#[test]
fn skipped_c_fixture_with_extension_owned_recipe_still_renders() {
    let mut fixture = documented_fixture();
    fixture.call = Some("clear_ocr_backends".into());
    fixture.skip = Some(crate::e2e::fixture::SkipDirective {
        languages: vec!["c".into()],
        reason: Some("The C API does not expose the clear call that pairs with registration".into()),
    });
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig::default();
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
        body: "extension_call()",
    })];
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &extensions)
            .expect("an extension-owned recipe renders even when the harness skips this language");

    assert_eq!(report.coverage.generated.len(), 1);
    assert!(report.coverage.missing.is_empty());
    assert_eq!(report.snippets.len(), 1);
    assert!(report.snippets[0].file.content.contains("extension_call()"));
}

/// A fixture that is not skipped for `c` still resolves its trait-bridge identity and
/// renders — the skip gate only suppresses the fallback for fixtures that opted out.
///
/// The assertions examine the RESULT BINDING, not just the call text. The version of this
/// test that shipped with the skip-gate fix asserted only
/// `contains("sample_clear_ocr_backend(NULL);")`, a substring that matches the tail of an
/// assignment line as happily as a standalone statement — so it passed while the emitter
/// bound the `i32` this export returns to `{PREFIX}AlefHandle` and then passed it to
/// `{prefix}__free`. A whole return type and a heap-corrupting free sat inside the span the
/// assertion did not look at. Substring checks on a call site are blind to everything to the
/// left of the symbol and everything on the following lines; both are pinned here. ~keep
#[test]
fn not_skipped_c_fixture_binds_the_trait_bridge_status_and_frees_nothing() {
    let mut fixture = documented_fixture();
    fixture.call = Some("clear_ocr_backends".into());
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".into(),
            clear_fn: Some("clear_ocr_backends".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &[])
        .expect("an unskipped fixture with a resolvable trait-bridge identity still generates a C snippet");

    assert_eq!(report.coverage.generated.len(), 1);
    assert!(report.coverage.missing.is_empty());
    assert_eq!(report.snippets.len(), 1);
    let content = &report.snippets[0].file.content;
    // The symbol is the one the FFI backend actually exports:
    // `{prefix}_clear_{trait_snake}` derived from the trait name (`registration.rs:141`),
    // SINGULAR — not the pluralised `clear_fn` config text, which only ever matched the
    // fixture to a bridge. The trailing `NULL` is the C out-error argument. The whole
    // statement is spelled out, so the declared type is inside what this examines. ~keep
    assert!(
        content.contains("int32_t result = sample_clear_ocr_backend(NULL);"),
        "expected the derived singular ABI symbol bound as the i32 status `clear_fn.jinja` \
         returns, got:\n{content}"
    );
    assert!(
        !content.contains("AlefHandle"),
        "an i32 status must never be bound to an opaque handle type, got:\n{content}"
    );
    assert!(
        !content.contains("free"),
        "a status code owns nothing, so the snippet must free nothing — this is the \
         heap-corruption half of the defect and no substring of the call site would show \
         it, got:\n{content}"
    );
}

/// The `NULL` a trait-bridge `clear`/`unregister` snippet emits must be the `out_error`
/// out-param appended from `extra_args` (`c.rs`, `clear_fn.jinja`), NOT a by-product of
/// rendering an absent fixture `input`. Those two sources were indistinguishable for as long
/// as the only fixture covering this path used `Fixture::default()`, whose `input` is
/// `Value::Null` -- and `json_to_c(Value::Null)` renders the literal `NULL`, landing in
/// exactly the out_error slot by coincidence. This fixture carries a NON-null `input`, so the
/// argument list can only read `(NULL)` if out_error is genuinely being appended: the
/// coincidence would emit the serialized input instead. ~keep
#[test]
fn trait_bridge_out_error_arg_comes_from_extra_args_not_from_a_null_fixture_input() {
    let mut fixture = documented_fixture();
    fixture.call = Some("clear_ocr_backends".into());
    fixture.input = serde_json::json!({"unused": "payload"});
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".into(),
            clear_fn: Some("clear_ocr_backends".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["c".into()], &snippet_config, &context, &[])
        .expect("a trait-bridge fixture with a non-null input still generates a C snippet");

    let content = &report.snippets[0].file.content;
    assert!(
        content.contains("sample_clear_ocr_backend(NULL);"),
        "out_error must be appended from extra_args regardless of the fixture input, got:\n{content}"
    );
    assert!(
        !content.contains("unused"),
        "the fixture input must not be spliced into the argument list, got:\n{content}"
    );
}

#[test]
fn shared_validation_identities_keep_distinct_target_output_paths() {
    let fixture = documented_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "call".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension {
        body: "extension_call()",
    })];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(
        &[fixture],
        &["node".into(), "wasm".into(), "kotlin".into(), "kotlin_android".into()],
        &snippet_config,
        &context,
        &extensions,
    )
    .expect("shared validation languages use distinct target output routes");

    let paths: BTreeSet<_> = report
        .snippets
        .iter()
        .map(|snippet| snippet.file.path.as_path())
        .collect();
    assert!(
        paths.contains(Path::new("docs/snippets/typescript/api/extension_owned.md")),
        "paths: {paths:?}"
    );
    assert!(
        paths.contains(Path::new("docs/snippets/wasm/api/extension_owned.md")),
        "paths: {paths:?}"
    );
    assert!(
        paths.contains(Path::new("docs/snippets/kotlin/api/extension_owned.md")),
        "paths: {paths:?}"
    );
    assert!(
        paths.contains(Path::new("docs/snippets/kotlin-android/api/extension_owned.md")),
        "paths: {paths:?}"
    );
    assert_eq!(report.coverage.generated, report.coverage.expected);
    for snippet in &report.snippets {
        let canonical = match snippet.language.as_str() {
            "node" | "wasm" => "typescript",
            "kotlin" | "kotlin_android" => "kotlin",
            other => panic!("unexpected target: {other}"),
        };
        assert!(snippet.file.content.contains(&format!("```{canonical} title=")));
        let metadata = report
            .coverage
            .generated_metadata
            .iter()
            .find(|metadata| metadata.key.language == snippet.language)
            .expect("target metadata");
        assert_eq!(metadata.language, canonical);
        assert_eq!(metadata.target, snippet.language);
        assert_eq!(metadata.session, snippet.language);
    }
}

#[test]
fn empty_extension_recipe_is_recorded_as_missing() {
    let fixture = documented_fixture();
    let mut e2e = E2eConfig::default();
    e2e.call.function = "call".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let extensions: Vec<Box<dyn crate::Extension>> = vec![Box::new(FixtureExtension { body: "  " })];
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &extensions)
            .expect("empty recipe belongs in coverage report");

    assert!(report.snippets.is_empty());
    assert_eq!(report.coverage.missing.len(), 1);
    assert!(report.coverage.missing[0].reason.contains("empty snippet body"));
}

#[test]
fn unsupported_brew_recipe_uses_exact_coverage_exception() {
    let mut fixture = documented_fixture();
    let docs = fixture.docs.as_mut().expect("fixture has documentation metadata");
    docs.coverage_exceptions.insert(
        "brew".into(),
        crate::e2e::fixture::SnippetCoverageException {
            reason: "The package installation flow is documented separately".into(),
            documentation: "docs/install/homebrew.md".into(),
        },
    );
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["brew".into()], &snippet_config, &context, &[])
        .expect("unsupported brew recipe belongs in coverage report");

    assert!(report.snippets.is_empty());
    assert!(report.coverage.missing.is_empty());
    assert_eq!(report.coverage.expected.len(), 1);
    assert_eq!(report.coverage.documented_exceptions.len(), 1);
    assert_eq!(report.coverage.documented_exceptions[0].key.language, "brew");
}

#[test]
fn unsupported_shell_targets_are_recorded_without_mapping_failures() {
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    for language in ["brew", "homebrew"] {
        let report = generate_snippet_report_with_extensions(
            &[documented_fixture()],
            &[language.into()],
            &snippet_config,
            &context,
            &[],
        )
        .expect("unsupported shell target belongs in coverage report");

        assert_eq!(report.coverage.expected.len(), 1);
        assert_eq!(report.coverage.missing.len(), 1);
        assert_eq!(report.coverage.missing[0].key.language, language);
        assert!(!report.coverage.missing[0].reason.is_empty());
        assert!(!report.coverage.missing[0].reason.contains("language mapping"));
    }
}

#[test]
fn fixture_without_docs_is_expected_and_recorded_as_missing() {
    let fixture = Fixture {
        id: "undocumented".into(),
        ..Fixture::default()
    };
    let e2e = E2eConfig::default();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report = generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &[])
        .expect("undocumented fixture belongs in coverage report");

    assert_eq!(report.coverage.expected.len(), 1);
    assert_eq!(report.coverage.missing.len(), 1);
    assert_eq!(
        report.coverage.missing[0].reason,
        "fixture has no documentation metadata"
    );
}

#[test]
fn rust_visitor_snippets_declare_the_required_feature() {
    let mut fixture = documented_fixture();
    fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
        callbacks: BTreeMap::new(),
    });

    assert_eq!(snippet_requirements(&fixture, "rust", ""), ["feature:visitor"]);
    assert!(snippet_requirements(&fixture, "java", "").is_empty());
}

fn json_argument_fixture() -> Fixture {
    let argument = |name: &str, arg_type: &str| crate::core::config::e2e::ArgMapping {
        name: name.into(),
        field: name.into(),
        arg_type: arg_type.into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    };
    Fixture {
        id: "json_options".into(),
        input: serde_json::json!({"text": "sample", "options": {"width": 80}}),
        args: vec![argument("text", "string"), argument("options", "json_object")],
        ..documented_fixture()
    }
}

fn rust_snippet_report(fixture: Fixture) -> SnippetGenerationReport {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let crate_config = ResolvedCrateConfig::default();
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };
    generate_snippet_report_with_extensions(&[fixture], &["rust".into()], &snippet_config, &context, &[])
        .expect("rust snippet report renders")
}

#[test]
fn rust_snippets_declare_the_serde_json_crate_their_body_names() {
    let report = rust_snippet_report(json_argument_fixture());

    let snippet = &report.snippets[0];
    assert!(
        snippet.file.content.contains("serde_json::from_str"),
        "body must exercise serde_json: {}",
        snippet.file.content
    );
    assert_eq!(snippet.requirements, ["crate:serde_json"]);
    assert!(
        snippet.file.content.contains("requires: [\"crate:serde_json\"]"),
        "frontmatter must declare the dependency: {}",
        snippet.file.content
    );
    assert_eq!(report.coverage.generated_metadata[0].requires, ["crate:serde_json"]);
}

#[test]
fn rust_snippets_without_json_arguments_declare_no_crate_requirement() {
    let report = rust_snippet_report(documented_fixture());

    let snippet = &report.snippets[0];
    assert!(!snippet.file.content.contains("serde_json"), "{}", snippet.file.content);
    assert!(snippet.requirements.is_empty());
}

/// An async fixture renders through `rust/snippet_body.rs.jinja`, which emits `#[tokio::main]`.
/// The snippet must carry the matching crate requirement, or the validator builds a check
/// project with no `tokio` in `[dependencies]` and the snippet fails on E0433 rather than on
/// anything it actually demonstrates.
#[test]
fn an_async_rust_snippet_requires_the_tokio_crate() {
    let body = "#[tokio::main]\nasync fn main() {\n    let value = 1u8;\n    println!(\"{value:?}\");\n}\n";

    let requirements = snippet_requirements(&documented_fixture(), "rust", body);

    assert_eq!(requirements, ["crate:tokio"], "async snippet must declare tokio");
}

#[test]
fn a_synchronous_rust_snippet_requires_no_tokio_crate() {
    let body = "fn main() {\n    let value = 1u8;\n    println!(\"{value:?}\");\n}\n";

    let requirements = snippet_requirements(&documented_fixture(), "rust", body);

    assert!(
        requirements.is_empty(),
        "a snippet with no tokio attribute must not pull tokio in: {requirements:?}"
    );
}
