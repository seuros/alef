use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Context, Result, bail};

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    if fixture.is_http_test() {
        bail!(
            "Dart documentation snippets do not support HTTP harness fixture `{}`",
            fixture.id
        );
    }
    let mut fixture_without_assertions = fixture.clone();
    fixture_without_assertions.assertions.clear();
    let bridge_class = config.dart_bridge_class_name();
    let first_class_map = super::values::build_dart_first_class_map(type_defs, enums, e2e_config);
    let mut test_case = String::new();
    super::test_case::render_test_case(
        &mut test_case,
        &fixture_without_assertions,
        super::test_case::DartTestCaseContext {
            e2e_config,
            lang: "dart",
            bridge_class: &bridge_class,
            dart_first_class_map: &first_class_map,
            adapters: &config.adapters,
            config,
            type_defs,
            enums,
        },
    );
    let statements = extract_test_statements(&test_case)
        .with_context(|| format!("extracting Dart snippet body for fixture `{}`", fixture.id))?;
    let package = e2e_config
        .resolve_package("dart")
        .and_then(|value| value.name)
        .unwrap_or_else(|| config.dart_pubspec_name());
    let module = config.name.replace('-', "_");
    let needs_json = statements
        .iter()
        .any(|statement| statement.contains("jsonDecode(") || statement.contains("jsonEncode("));
    Ok(crate::e2e::template_env::render(
        "dart/snippet_body.jinja",
        minijinja::context! {
            package => package, module => module, statements => statements, needs_json => needs_json,
        },
    ))
}

fn extract_test_statements(rendered: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = rendered.lines().collect();
    let start = lines.iter().position(|line| line.trim_start().starts_with("test("))? + 1;
    let end = lines.iter().rposition(|line| line.trim() == "});")?;
    Some(
        lines[start..end]
            .iter()
            .map(|line| line.strip_prefix("    ").unwrap_or(line).to_string())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_the_native_test_body() {
        let rendered = "  test('sample', () async {\n    final value = await Api.load();\n  });\n";
        assert_eq!(
            extract_test_statements(rendered),
            Some(vec!["final value = await Api.load();".to_string()])
        );
    }

    #[test]
    fn renders_native_call_without_test_harness() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();
        assert!(body.contains("loadDocument()"));
        assert!(body.contains("Future<void> main() async"));
        assert!(!body.contains("test("));
        assert!(!body.contains("expect("));
    }
}
