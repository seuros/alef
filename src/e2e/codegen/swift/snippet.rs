use super::{test_method, values};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};
use heck::ToUpperCamelCase;

pub(super) fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Result<String> {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if fixture.http.is_some() || fixture.visitor.is_some() || call.streaming_enabled() == Some(true) {
        bail!(
            "swift snippet `{}` requires an unsupported HTTP, visitor, or streaming call pattern",
            fixture.id
        );
    }
    if fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error")
    {
        bail!(
            "swift snippet `{}` cannot represent an expected-error fixture",
            fixture.id
        );
    }
    if call.args.iter().any(|argument| argument.arg_type == "test_backend") {
        bail!(
            "swift snippet `{}` requires test-backend lifecycle teardown",
            fixture.id
        );
    }

    let package = e2e_config
        .resolve_package("swift")
        .and_then(|package| package.name)
        .unwrap_or_else(|| config.name.to_upper_camel_case());
    let module = package.to_upper_camel_case();
    let first_class_map = values::build_swift_first_class_map(type_defs, enums, e2e_config, call);
    let override_config = call.overrides.get("swift");
    let mut call_fixture = fixture.clone();
    call_fixture.assertions.clear();
    let mut method = String::new();
    test_method::render_test_method(
        &mut method,
        &call_fixture,
        e2e_config,
        "",
        "",
        &[],
        false,
        override_config.and_then(|value| value.client_factory.as_deref()),
        &first_class_map,
        &module,
        config,
        type_defs,
        enums,
    );
    let body = method
        .lines()
        .skip(2)
        .take_while(|line| line.trim() != "}")
        .map(|line| line.strip_prefix("        ").unwrap_or(line))
        .map(|line| line.replacen("let  =", "_ =", 1))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(crate::e2e::template_env::render(
        "swift/snippet_body.jinja",
        minijinja::context! { module => module, body => body },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_reuses_typed_call_without_xctest_harness() {
        let fixture = Fixture {
            id: "count".into(),
            description: "Count".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "count_items".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let rendered = render(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");
        assert!(rendered.contains("import RustBridge"));
        assert!(rendered.contains("_ = try "));
        assert!(rendered.contains(".countItems()"));
        assert!(!rendered.contains("XCTest"));
    }
}
