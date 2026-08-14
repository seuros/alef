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
    let options_ptr = override_config.is_some_and(|value| value.options_ptr)
        || call.overrides.get(lang).is_some_and(|value| value.options_ptr)
        || e2e_config
            .call
            .overrides
            .get(lang)
            .is_some_and(|value| value.options_ptr);
    let options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|value| value.options_type.as_deref())
    });
    let (mut package_decls, mut setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        import_alias,
        options_type,
        fixture,
        options_ptr,
        false,
        &data_enum_names,
        config,
        type_defs,
        enums,
        true,
    );
    if let Some(visitor_spec) = &fixture.visitor
        && let Some(options_type) =
            options_type.or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
    {
        let struct_name = super::visitors::visitor_struct_name(&fixture.id);
        let binding = super::visitors::resolve_go_visitor_binding(config, type_defs, visitor_spec, import_alias);
        let mut declaration = String::new();
        super::visitors::emit_go_visitor_struct(
            &mut declaration,
            &struct_name,
            visitor_spec,
            import_alias,
            binding.as_ref(),
        );
        package_decls.push(declaration);
        setup_lines.push(format!("visitor := &{struct_name}{{}}"));
        setup_lines.push(format!("opts := &{import_alias}.{options_type}{{}}"));
        setup_lines.push("opts.Visitor = visitor".to_string());
        args = replace_go_options(&args);
    }
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
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let mut standard_imports = std::collections::BTreeSet::new();
    let setup_lines: Vec<String> = setup_lines.into_iter().map(snippet_setup_line).collect();
    let joined_setup = setup_lines.join("\n");
    let joined_declarations = package_decls.join("\n");
    if joined_setup.contains("os.") || joined_declarations.contains("os.") {
        standard_imports.insert("os");
    }
    if joined_setup.contains("json.") {
        standard_imports.insert("encoding/json");
    }
    if joined_setup.contains("strings.") {
        standard_imports.insert("strings");
    }
    if !call.returns_void || expects_error || joined_setup.contains("fmt.") {
        standard_imports.insert("fmt");
    }
    if expects_error {
        standard_imports.insert("errors");
        standard_imports.insert("os");
    }
    let mut imports = standard_imports
        .into_iter()
        .map(|path| (path.to_string(), String::new()))
        .collect::<Vec<_>>();
    imports.push((module.to_string(), import_alias.to_string()));
    imports.sort_by(|left, right| left.0.cmp(&right.0));
    let imports = imports
        .into_iter()
        .map(|(path, alias)| minijinja::context! { path => path, alias => alias })
        .collect::<Vec<_>>();

    crate::e2e::template_env::render(
        "go/snippet_body.jinja",
        minijinja::context! {
            imports => imports,
            package_decls => package_decls, setup_lines => setup_lines, client_setup => client_setup,
            call_expr => call_expr, result_var => call.result_var, returns_error => returns_error,
            returns_void => call.returns_void,
            expects_error => expects_error,
            error_type => crate::e2e::codegen::snippet_error_type_name(config),
            import_alias => import_alias,
        },
    )
    .trim_end()
    .to_string()
}

fn replace_go_options(args: &str) -> String {
    if let Some(prefix) = args.strip_suffix(", nil") {
        format!("{prefix}, opts")
    } else if args.is_empty() {
        "opts".to_string()
    } else {
        format!("{args}, opts")
    }
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

    #[test]
    fn visitor_options_replace_nil_argument() {
        assert_eq!(replace_go_options("html, nil"), "html, opts");
        assert_eq!(replace_go_options("html"), "html, opts");
    }
    use crate::e2e::config::{CallConfig, CallOverride};

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
            preserve_input_urls: false,
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
    fn snippet_renders_expected_error_as_an_executable_example() {
        let mut fixture = fixture();
        fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();
        e2e.call.returns_result = true;
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]);

        assert!(body.contains("_, err := pkg."), "{body}");
        assert!(body.contains("var typedError *pkg.Error"), "{body}");
        assert!(body.contains("errors.As(err, &typedError)"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
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

    #[test]
    fn snippet_separates_package_and_import_declarations() {
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();

        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[]);

        assert!(body.starts_with("package main\n\nimport (\n"), "{body}");
        assert!(!body.contains("package main import"), "{body}");
    }

    #[test]
    fn snippet_matches_gofmt_when_available() {
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();
        e2e.call.function = "process".into();
        e2e.call.result_var = "result".into();
        e2e.call.returns_result = true;
        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[]);
        let Ok(mut child) = std::process::Command::new("gofmt")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            assert!(body.contains("\tresult, err := pkg.Process()"), "{body}");
            return;
        };
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("gofmt stdin")
            .write_all(body.as_bytes())
            .expect("write Go snippet");
        let output = child.wait_with_output().expect("wait for gofmt");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            String::from_utf8(output.stdout)
                .expect("gofmt output is UTF-8")
                .trim_end(),
            body
        );
    }

    #[test]
    fn snippet_constructs_known_dto_without_json_round_trip() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"payload": {"kind": "active", "label": "sample"}});
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();
        e2e.call.function = "process".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = [
            ("payload", "input.payload", Some("SampleInput")),
            ("config", "input.config", None),
        ]
        .into_iter()
        .map(|(name, field, element_type)| crate::e2e::config::ArgMapping {
            name: name.into(),
            field: field.into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: element_type.map(str::to_string),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        })
        .collect();
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                options_ptr: true,
                options_type: Some("SampleConfig".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[
                TypeDef {
                    name: "SampleInput".into(),
                    fields: vec![
                        crate::core::ir::FieldDef {
                            name: "kind".into(),
                            ty: crate::core::ir::TypeRef::Named("SampleKind".into()),
                            default: Some("active".into()),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "label".into(),
                            ty: crate::core::ir::TypeRef::String,
                            ..Default::default()
                        },
                    ],
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "SampleConfig".into(),
                    ..TypeDef::default()
                },
            ],
            &[],
        );

        assert!(
            body.contains(
                "payload := pkg.SampleInput{\n\t\tKind:  ptr(pkg.SampleKind(`active`)),\n\t\tLabel: `sample`,"
            ),
            "{body}"
        );
        assert!(body.contains("config := pkg.SampleConfig{}"), "{body}");
        assert!(body.contains("pkg.Process(payload, config)"), "{body}");
        assert!(!body.contains("pkg.Process(payload, nil)"), "{body}");
        assert!(!body.contains("json.Unmarshal"), "{body}");
        assert!(!body.contains("encoding/json"), "{body}");
    }

    #[test]
    fn snippet_honors_shared_options_pointer_and_prints_fields() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "options": {} });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "options".into(),
            field: "options".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "go".into(),
            crate::e2e::config::CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                ..Default::default()
            },
        );
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
        );

        assert!(rendered.contains("&options"), "{rendered}");
        assert!(rendered.contains("fmt.Printf(\"%+v\\n\", result)"), "{rendered}");
    }
}
