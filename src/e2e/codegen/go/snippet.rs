use crate::codegen::naming::{go_free_function_name, go_type_name, to_go_name};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::setup::build_args_and_setup;

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> String {
    let lang = "go";
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs);
    let override_config = recipe.override_config;
    let import_alias = override_config
        .and_then(|value| value.alias.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|value| value.alias.as_deref())
        })
        .unwrap_or("pkg");
    let module = override_config
        .and_then(|value| value.module.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|value| value.module.as_deref())
        })
        .or_else(|| config.go.as_ref().and_then(|value| value.module.as_deref()))
        .unwrap_or(&call.module);
    let reserved_type_names: std::collections::HashSet<String> = type_defs
        .iter()
        .filter(|value| !value.is_trait)
        .map(|value| go_type_name(&value.name))
        .chain(enums.iter().map(|value| go_type_name(&value.name)))
        .collect();
    let base_function = override_config
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function);
    let function_name = go_free_function_name(base_function, &reserved_type_names);
    let data_enum_names: std::collections::HashSet<&str> = enums
        .iter()
        .filter(|value| {
            value
                .variants
                .iter()
                .any(|variant| variant.fields.iter().any(|field| !field.name.is_empty()))
        })
        .map(|value| value.name.as_str())
        .collect();
    let options_ptr = override_config.map(|value| value.options_ptr).unwrap_or(false);
    let (package_decls, setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        import_alias,
        recipe.options_type,
        fixture,
        options_ptr,
        false,
        &data_enum_names,
        config,
        type_defs,
        enums,
    );
    if !recipe.extra_args.is_empty() {
        let extras = recipe.extra_args.join(", ");
        args = if args.is_empty() {
            extras
        } else {
            format!("{args}, {extras}")
        };
    }
    let client_factory = override_config
        .and_then(|value| value.client_factory.as_deref())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|value| value.client_factory.as_deref())
        });
    let (call_prefix, client_setup) = if let Some(factory) = client_factory {
        (
            "client".to_string(),
            format!(
                "\tclient, clientErr := {import_alias}.{}(\"your-api-key\", nil, nil, nil, nil)\n\tif clientErr != nil {{\n\t\tpanic(clientErr)\n\t}}",
                to_go_name(factory),
            ),
        )
    } else {
        (import_alias.to_string(), String::new())
    };
    let call_expr = format!("{call_prefix}.{function_name}({args})");
    let returns_error = override_config
        .and_then(|value| value.returns_result)
        .unwrap_or(call.returns_result)
        || recipe
            .args
            .iter()
            .any(|arg| matches!(arg.arg_type.as_str(), "json_object" | "bytes"))
        || client_factory.is_some();
    let mut standard_imports = std::collections::BTreeSet::new();
    let setup_lines: Vec<String> = setup_lines.into_iter().map(snippet_setup_line).collect();
    let joined_setup = setup_lines.join("\n");
    if joined_setup.contains("os.") {
        standard_imports.insert("os");
    }
    if joined_setup.contains("json.") {
        standard_imports.insert("encoding/json");
    }
    if joined_setup.contains("strings.") {
        standard_imports.insert("strings");
    }
    if !call.returns_void || joined_setup.contains("fmt.") {
        standard_imports.insert("fmt");
    }

    crate::e2e::template_env::render(
        "go/snippet_body.jinja",
        minijinja::context! {
            module => module, import_alias => import_alias, standard_imports => standard_imports,
            package_decls => package_decls, setup_lines => setup_lines, client_setup => client_setup,
            call_expr => call_expr, result_var => call.result_var, returns_error => returns_error,
            returns_void => call.returns_void,
        },
    )
}

fn snippet_setup_line(line: String) -> String {
    line.lines()
        .map(|part| {
            if part.contains("t.Fatalf(") {
                format!("{})", part.replace("t.Fatalf(", "panic(fmt.Sprintf("))
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::CallConfig;

    fn fixture() -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "quick_start".to_string(),
            category: None,
            description: "Quick start".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            assertions: Vec::new(),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
        }
    }

    #[test]
    fn snippet_reuses_the_test_call_shape_without_test_harness() {
        let e2e = E2eConfig {
            call: CallConfig {
                function: "load_document".to_string(),
                module: "github.com/example/library".to_string(),
                result_var: "document".to_string(),
                returns_result: true,
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };
        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[]);

        assert!(body.contains("pkg \"github.com/example/library\""));
        let fmt_position = body.find("\"fmt\"").expect("fmt import");
        let package_position = body.find("pkg \"").expect("binding import");
        assert!(fmt_position < package_position, "{body}");
        assert!(body.contains("document, err := pkg.LoadDocument()"));
        assert!(!body.contains("testing"));
        assert!(!body.contains("assert."));
    }

    #[test]
    fn snippet_replaces_testing_failures_in_typed_setup() {
        assert_eq!(
            snippet_setup_line("if err != nil {\n\tt.Fatalf(\"decode: %v\", err)\n}".into()),
            "if err != nil {\n\tpanic(fmt.Sprintf(\"decode: %v\", err))\n}"
        );
    }

    #[test]
    fn void_snippet_does_not_import_fmt_when_it_is_unused() {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "reset".into();
        e2e.call.module = "github.com/example/library".into();
        e2e.call.returns_void = true;

        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[]);

        assert!(!body.contains("\"fmt\""), "{body}");
    }
}
