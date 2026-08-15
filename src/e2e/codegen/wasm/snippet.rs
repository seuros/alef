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
    if !functions.is_empty() && !crate::backends::wasm::function_is_exported(&call.function, functions, config) {
        bail!(
            "WASM target does not export the configured `{}` fixture function",
            call.function
        );
    }
    let overrides = e2e_config.call.overrides.get("wasm");
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
            client_factory: overrides.and_then(|value| value.client_factory.as_deref()),
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
        let fixture = Fixture {
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

        e2e.call.function = "prefetch".into();
        let available = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect("enabled function renders");
        assert!(available.contains("import { prefetch }"), "{available}");
    }
}
