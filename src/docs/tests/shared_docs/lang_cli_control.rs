//! `--lang c` / `--lang jni` CLI-resolver regression tests for the shared, language-neutral
//! docs pages -- split out of `shared_docs.rs` to stay under the file-modularization line cap
//! (see that file's own cfg-union regression group, which this reuses).

use super::*;

/// Resolve `--lang c` through the real CLI language resolver (the same `resolve_doc_languages`
/// `alef docs --lang c` runs) and assert it lands on the single-language path
/// `test_shared_pages_use_configured_union_for_the_lang_c_cli_path` exercises. This is a
/// precondition/sanity check on the CLI resolver, not the regression assertion itself -- that
/// test's own `Turbo`/`NeverShipped` checks on the rendered page are what actually distinguish
/// the fix. ~keep
fn resolve_lang_c(config: &ResolvedCrateConfig) -> Vec<Language> {
    let lang_filter: Option<Vec<String>> = Some(vec!["c".to_string()]);
    let rendered = crate::bin_cli::helpers::resolve_doc_languages(config, lang_filter.as_deref()).unwrap();
    assert_eq!(
        rendered,
        vec![Language::C],
        "sanity check: `--lang c` must resolve to exactly [C], the path this test exercises"
    );
    rendered
}

/// Same as [`resolve_lang_c`], for `--lang jni` -- kept as its own small function rather than a
/// parameterized shared one: `generate_docs` and `canonical_docs_api` skip `C`/`Jni` via the
/// identical `matches!(lang, Language::C | Language::Jni)` arm, but "shares a code path" is an
/// argument, not a test, so `test_shared_pages_use_configured_union_for_the_lang_jni_cli_path`
/// exercises `Jni` through this real resolver call rather than assuming it behaves like `C`. ~keep
fn resolve_lang_jni(config: &ResolvedCrateConfig) -> Vec<Language> {
    let lang_filter: Option<Vec<String>> = Some(vec!["jni".to_string()]);
    let rendered = crate::bin_cli::helpers::resolve_doc_languages(config, lang_filter.as_deref()).unwrap();
    assert_eq!(
        rendered,
        vec![Language::Jni],
        "sanity check: `--lang jni` must resolve to exactly [Jni], the path this test exercises"
    );
    rendered
}

/// Assert `generate_docs` wrote no per-language `api-*.md` page -- expected whenever the
/// rendered language set is entirely `C`/`Jni` (both always skipped by the render loop). ~keep
fn assert_no_lang_pages_rendered(files: &[GeneratedFile]) {
    assert!(
        !files.iter().any(|f| f.path.to_str().unwrap().starts_with("out/api-")),
        "no per-language page should render for `--lang c` alone"
    );
}

/// Non-vacuous regression for the `alef docs --lang c` CLI path specifically (required by the
/// follow-up review, not just the general config-vs-rendered shape above): this crate configures
/// `python`, `wasm`, AND `c` (a real, resolvable `--lang c` target), with only `wasm` enabling
/// `acceleration`. Resolving `--lang c` through the actual CLI language resolver
/// (`resolve_doc_languages`, the same function `alef docs --lang c` runs) yields a rendered set
/// of `[C]` alone -- `generate_docs`'s render loop always skips `C`/`Jni`, so ZERO per-language
/// pages get written this invocation. The shared pages must still reflect the full configured
/// union (`Turbo` present via wasm) and must still exclude a feature no configured language
/// enables (`NeverShipped`, absent) -- a rendered-set-sourced union would see an empty rendered
/// set (after the `C` skip) and fall back to the unfiltered surface, which would wrongly let
/// `NeverShipped` leak through too. `NeverShipped`'s absence is what makes this test fail against
/// that pre-follow-up shape; `Turbo`'s presence alone would not (both shapes keep it).
///
/// See [`test_shared_pages_use_configured_union_for_the_lang_jni_cli_path`] below for the same
/// control on `--lang jni` -- `C` and `Jni` share the identical skip arm in both `generate_docs`
/// and `canonical_docs_api`, but that is an argument for why `Jni` is *likely* to behave the same,
/// not a substitute for exercising it through the real resolver too. ~keep
#[test]
fn test_shared_pages_use_configured_union_for_the_lang_c_cli_path() {
    let api = api_with_enum(pipeline_mode_enum(&[
        PipelineVariantSpec {
            name: "Turbo",
            doc: "Multi-threaded accelerated mode.",
            cfg_feature: Some("acceleration"),
            is_default: false,
        },
        PipelineVariantSpec {
            name: "NeverShipped",
            doc: "Not enabled by any configured language.",
            cfg_feature: Some("never-enabled"),
            is_default: false,
        },
    ]));

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python", "wasm", "c"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.wasm]
features = ["acceleration"]
"#,
    );

    let rendered = resolve_lang_c(&config);
    let files = generate_docs(&api, &config, &rendered, "out").unwrap();
    assert_no_lang_pages_rendered(&files);
    let types_file = shared_page_content(&files, "types");

    assert!(
        types_file.contains("`Turbo`"),
        "`--lang c` must still describe the full configured surface (wasm enables this variant); \
         got:\n{types_file}"
    );
    assert!(
        !types_file.contains("NeverShipped"),
        "`--lang c` must not fall back to the unfiltered surface just because no per-language \
         page rendered this invocation -- the canonical union must come from config.languages, \
         which is non-empty (python, wasm) here; got:\n{types_file}"
    );
}

/// The `--lang jni` control for
/// [`test_shared_pages_use_configured_union_for_the_lang_c_cli_path`]: same setup shape, but
/// resolved through the real CLI resolver as `Jni` rather than `C`, and the config additionally
/// configures `kotlin_android` (required by `jni`'s own resolve-time validation: a crate
/// configuring `jni` without also configuring `kotlin_android` fails to resolve). `C` and `Jni`
/// take the identical `matches!(lang, Language::C | Language::Jni)` skip arm in both
/// `generate_docs` and `canonical_docs_api`, so this is expected to behave identically to the `C`
/// case above -- but "shares a code path" is an argument, not a test, so this exercises `Jni`
/// through `resolve_doc_languages` directly instead of assuming it. ~keep
#[test]
fn test_shared_pages_use_configured_union_for_the_lang_jni_cli_path() {
    let api = api_with_enum(pipeline_mode_enum(&[
        PipelineVariantSpec {
            name: "Turbo",
            doc: "Multi-threaded accelerated mode.",
            cfg_feature: Some("acceleration"),
            is_default: false,
        },
        PipelineVariantSpec {
            name: "NeverShipped",
            doc: "Not enabled by any configured language.",
            cfg_feature: Some("never-enabled"),
            is_default: false,
        },
    ]));

    let config = config_from_toml(
        r#"
[workspace]
languages = ["python", "wasm", "jni", "kotlin_android"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.wasm]
features = ["acceleration"]
"#,
    );

    let rendered = resolve_lang_jni(&config);
    let files = generate_docs(&api, &config, &rendered, "out").unwrap();
    assert_no_lang_pages_rendered(&files);
    let types_file = shared_page_content(&files, "types");

    assert!(
        types_file.contains("`Turbo`"),
        "`--lang jni` must still describe the full configured surface (wasm enables this \
         variant); got:\n{types_file}"
    );
    assert!(
        !types_file.contains("NeverShipped"),
        "`--lang jni` must not fall back to the unfiltered surface just because no per-language \
         page rendered this invocation -- the canonical union must come from config.languages, \
         which is non-empty (python, wasm) here; got:\n{types_file}"
    );
}
