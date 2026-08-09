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
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
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
    if !expects_error {
        call_fixture.assertions.clear();
    }
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
    let body_line_count = method.lines().count().saturating_sub(3);
    let body = method
        .lines()
        .skip(2)
        .take(body_line_count)
        .map(|line| line.strip_prefix("        ").unwrap_or(line))
        .map(|line| line.replacen("let  =", "_ =", 1))
        .map(|line| {
            line.replace(
                "XCTFail(\"expected to throw\")",
                "fatalError(\"expected call to fail\")",
            )
        })
        .map(|line| line.replace("// success", "print(\"Call failed as expected: \\(error)\")"))
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

    #[test]
    fn expected_error_snippet_uses_native_do_catch() {
        let mut fixture = Fixture {
            id: "invalid".into(),
            description: "Invalid".into(),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "parse".into();
        let rendered = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet renders");
        assert!(rendered.contains("do {"));
        assert!(rendered.contains("catch {"));
        assert!(rendered.contains("Call failed as expected"));
        assert!(!rendered.contains("XCTFail"));
    }
}
