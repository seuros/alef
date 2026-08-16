use anyhow::{Result, bail};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, FunctionDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

pub(super) fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> Result<String> {
    let docs_fixture = fixture.docs_call_fixture();
    let call = e2e_config.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, &docs_fixture);
    let default_factory = e2e_config
        .call
        .overrides
        .get("wasm")
        .and_then(|value| value.client_factory.as_deref());
    let effective_factory = call
        .overrides
        .get("wasm")
        .and_then(|value| value.client_factory.as_deref())
        .or(default_factory);
    let Some(function) = call.effective_function("wasm") else {
        bail!(
            "call routed for fixture `{}` has no function identity for WASM: neither the call's \
             base `function` nor `overrides.wasm.function` supplies one",
            docs_fixture.id
        );
    };
    if effective_factory.is_none() && !functions.is_empty() {
        match crate::backends::wasm::wasm_callability(function, functions, config) {
            crate::backends::wasm::WasmCallability::Callable => {}
            crate::backends::wasm::WasmCallability::NotExported => {
                bail!("WASM target does not export the configured `{function}` fixture function");
            }
            crate::backends::wasm::WasmCallability::UnknownSymbol => {
                bail!(
                    "fixture `{}` routes WASM to `{function}`, but nothing in the API surface or the \
                     trait-bridge registry answers to that name under either its Rust or its JavaScript \
                     spelling -- the name resolves to nothing, which is a config error rather than a gap \
                     in the WASM target",
                    docs_fixture.id
                );
            }
        }
    }
    let module = e2e_config
        .resolve_package("wasm")
        .and_then(|package| package.name)
        .unwrap_or_else(|| config.wasm_package_name());
    let wasm_type_prefix = config.wasm_type_prefix();
    Ok(super::super::typescript::test_file::render_snippet_body(
        super::super::typescript::test_file::SnippetContext {
            lang: "wasm",
            fixture,
            module: &module,
            client_factory: effective_factory,
            e2e_config,
            type_defs,
            enums,
            wasm_type_prefix: &wasm_type_prefix,
            config,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_follow_the_wasm_function_surface() {
        let mut fixture = Fixture {
            id: "download_assets".into(),
            description: "Download assets".into(),
            ..Fixture::default()
        };
        let functions = vec![
            FunctionDef {
                name: "download".into(),
                rust_path: "sample::download".into(),
                cfg: Some(r#"feature = "download""#.into()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "prefetch".into(),
                rust_path: "sample::prefetch".into(),
                cfg: Some(r#"not(feature = "download")"#.into()),
                ..FunctionDef::default()
            },
        ];
        let mut e2e = E2eConfig::default();
        e2e.call.function = "download".into();

        let unavailable = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("disabled function must not produce a WASM snippet");
        assert!(unavailable.to_string().contains("does not export"));

        e2e.calls.insert(
            "wrapped_download".into(),
            crate::e2e::config::CallConfig {
                function: "download".into(),
                overrides: std::iter::once((
                    "wasm".into(),
                    crate::core::config::e2e::CallOverride {
                        client_factory: Some("createClient".into()),
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
        );
        fixture.call = Some("wrapped_download".into());
        let client_recipe = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect("client method need not be a direct module export");
        assert!(client_recipe.contains("import { createClient }"), "{client_recipe}");
        assert!(client_recipe.contains("client.download("), "{client_recipe}");
        assert!(!client_recipe.contains("import { download }"), "{client_recipe}");

        fixture.call = None;
        e2e.call.function = "prefetch".into();
        let available = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect("enabled function renders");
        assert!(available.contains("import { prefetch }"), "{available}");
    }

    #[test]
    fn trait_bridge_registry_calls_are_callable_from_wasm_snippets() {
        // A bridge's register/unregister/clear functions are absent from the plain function
        // surface by construction -- the trait-bridge generator emits them into
        // `__alef_wasm_bridge_*` instead. Gating snippets on the codegen predicate therefore
        // rejects every registry operation even though WASM exports all of them.
        let fixture = Fixture {
            id: "clear_validators".into(),
            description: "Clear all validators".into(),
            ..Fixture::default()
        };
        let unrelated = vec![FunctionDef {
            name: "extract".into(),
            rust_path: "sample::extract".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.call.function = "clear_validators".into();
        let config = ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "Validator".into(),
                clear_fn: Some("clear_validators".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render(&fixture, &e2e, &config, &[], &[], &unrelated)
            .expect("a bridge-managed registry call is exported by the WASM bridge module");
        assert!(rendered.contains("clearValidators"), "{rendered}");
    }

    #[test]
    fn a_call_without_any_function_identity_is_reported_as_such() {
        // An empty name must never reach the export check: it renders as an empty identifier and
        // reads as a capability gap when the real fault is a call with no identity configured.
        let fixture = Fixture {
            id: "clear_validators".into(),
            description: "Clear all validators".into(),
            ..Fixture::default()
        };
        let functions = vec![FunctionDef {
            name: "extract".into(),
            rust_path: "sample::extract".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.call.function = String::new();

        let error = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("a call with no function identity cannot render");
        let error = error.to_string();
        assert!(error.contains("has no function identity"), "{error}");
        assert!(!error.contains("does not export"), "{error}");
    }

    /// Build the call shape a crate uses when its bindings disagree on the spelling and there is
    /// no language-neutral name to put at the base: `function = ""` plus one override per
    /// language. `clear_reranker_backends` in xberg's `alef.toml` is exactly this.
    fn call_named_only_by_overrides(overrides: &[(&str, &str)]) -> crate::e2e::config::CallConfig {
        crate::e2e::config::CallConfig {
            function: String::new(),
            overrides: overrides
                .iter()
                .map(|(lang, function)| {
                    (
                        (*lang).to_string(),
                        crate::core::config::e2e::CallOverride {
                            function: Some((*function).to_string()),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    fn reranker_bridge_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "RerankerBackend".into(),
                clear_fn: Some("clear_reranker_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    #[test]
    fn a_wasm_override_supplies_the_function_identity_the_empty_base_lacks() {
        let mut fixture = Fixture {
            id: "reranker_backends_clear".into(),
            description: "Clear all reranker backends".into(),
            ..Fixture::default()
        };
        fixture.call = Some("clear_reranker_backends".into());
        let unrelated = vec![FunctionDef {
            name: "rerank".into(),
            rust_path: "sample::rerank".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_reranker_backends".into(),
            call_named_only_by_overrides(&[("wasm", "clearRerankerBackends")]),
        );

        let rendered = render(&fixture, &e2e, &reranker_bridge_config(), &[], &[], &unrelated)
            .expect("the wasm override names a function the wasm bridge module exports");
        assert!(rendered.contains("clearRerankerBackends()"), "{rendered}");
    }

    #[test]
    fn an_override_for_another_language_does_not_give_wasm_an_identity() {
        let mut fixture = Fixture {
            id: "reranker_backends_clear".into(),
            description: "Clear all reranker backends".into(),
            ..Fixture::default()
        };
        fixture.call = Some("clear_reranker_backends".into());
        let unrelated = vec![FunctionDef {
            name: "rerank".into(),
            rust_path: "sample::rerank".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_reranker_backends".into(),
            call_named_only_by_overrides(&[("python", "clear_reranker_backends")]),
        );

        let error = render(&fixture, &e2e, &reranker_bridge_config(), &[], &[], &unrelated)
            .expect_err("an override for python says nothing about what wasm exports")
            .to_string();
        assert!(error.contains("has no function identity"), "{error}");
        assert!(!error.contains("does not export"), "{error}");
    }

    #[test]
    fn a_wasm_override_naming_an_unexported_function_still_fails() {
        let mut fixture = Fixture {
            id: "download_assets".into(),
            description: "Download assets".into(),
            ..Fixture::default()
        };
        fixture.call = Some("download".into());
        let functions = vec![
            FunctionDef {
                name: "download".into(),
                rust_path: "sample::download".into(),
                cfg: Some(r#"feature = "download""#.into()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "prefetch".into(),
                rust_path: "sample::prefetch".into(),
                cfg: Some(r#"not(feature = "download")"#.into()),
                ..FunctionDef::default()
            },
        ];
        let mut e2e = E2eConfig::default();
        e2e.calls
            .insert("download".into(), call_named_only_by_overrides(&[("wasm", "download")]));

        let gated = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("resolving the override must not excuse a function the target drops")
            .to_string();
        assert!(gated.contains("does not export"), "{gated}");
        assert!(gated.contains("`download`"), "{gated}");

        e2e.calls
            .insert("download".into(), call_named_only_by_overrides(&[("wasm", "fetchAssets")]));
        let absent = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("a name nothing answers to under either spelling is not callable")
            .to_string();
        assert!(absent.contains("`fetchAssets`"), "{absent}");
        assert!(
            absent.contains("answers to that name"),
            "a name that resolves to nothing is a config error, not a WASM capability gap: {absent}"
        );
        assert!(!absent.contains("does not export"), "{absent}");
    }
}
