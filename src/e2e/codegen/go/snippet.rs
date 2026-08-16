use crate::codegen::naming::{go_free_function_name, go_type_name, to_go_name};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};

use super::setup::build_args_and_setup;

/// Unwraps a (possibly `Optional`-wrapped) `TypeRef::Named` down to its type name.
///
/// Used to match a function parameter's IR type against the configured `options_type`
/// name so the pointer-vs-value and arity decisions below can be derived from the
/// same signature the Go binding backend generated from, instead of re-asserting it
/// independently. ~keep
fn go_ir_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => go_ir_named_type(inner),
        _ => None,
    }
}

/// Mirrors `backends::go::gen_bindings::functions::gen_function_wrapper`'s pointer-vs-value
/// decision for a non-bridge parameter: the Go binding backend emits `*T` when the IR
/// parameter is `optional`, or when its `Named` type is opaque — value `T` otherwise. Both
/// this function and the binding backend read the same `ParamDef`/`TypeDef.is_opaque`
/// facts; this is a re-derivation of the same public inputs; it is not a copy of any
/// gen_bindings-private logic. ~keep
fn go_options_param_is_pointer(param: &ParamDef, opaque_names: &std::collections::HashSet<&str>) -> bool {
    if param.optional {
        return true;
    }
    matches!(&param.ty, TypeRef::Named(name) if opaque_names.contains(name.as_str()))
}

/// Mirrors `gen_bindings::functions::is_bridge_param`'s two membership checks (by
/// parameter name, then by `Named` type alias) using the same `TraitBridgeConfig` facts
/// the binding backend reads — the params those checks match are real Rust function
/// parameters that the Go binding backend strips from its emitted signature (replaced by
/// a `nil` argument at the FFI call site), so they must not be counted toward the
/// Go-visible arity used by the `extra_args` clamp below. ~keep
fn go_is_bridge_param(
    param: &ParamDef,
    bridge_param_names: &std::collections::HashSet<String>,
    bridge_type_aliases: &std::collections::HashSet<String>,
) -> bool {
    if bridge_param_names.contains(&param.name) {
        return true;
    }
    go_ir_named_type(&param.ty).is_some_and(|name| bridge_type_aliases.contains(name))
}

pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> Result<String> {
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
    let options_type = recipe.options_type.or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|value| value.options_type.as_deref())
    });
    // `functions` is the same IR the Go binding backend generated the actual signature
    // from (see `gen_bindings::functions::gen_function_wrapper`). When the target call
    // resolves to a known free function, derive `options_ptr` from its real parameter
    // instead of trusting the hand-authored `options_ptr` override, which can drift from
    // what the binding backend emits. The override remains the fallback for calls this
    // harness cannot resolve to a `FunctionDef` (e.g. method calls, synthetic call
    // names) — those keep today's config-driven behavior unchanged. ~keep
    let target_function = functions.iter().find(|value| value.name == call.function);
    let opaque_names: std::collections::HashSet<&str> = type_defs
        .iter()
        .filter(|value| value.is_opaque)
        .map(|value| value.name.as_str())
        .collect();
    let options_param = target_function.and_then(|function| {
        function
            .params
            .iter()
            .find(|param| go_ir_named_type(&param.ty) == options_type)
    });
    let options_ptr = options_param
        .map(|param| go_options_param_is_pointer(param, &opaque_names))
        .unwrap_or_else(|| {
            override_config.is_some_and(|value| value.options_ptr)
                || call.overrides.get(lang).is_some_and(|value| value.options_ptr)
                || e2e_config
                    .call
                    .overrides
                    .get(lang)
                    .is_some_and(|value| value.options_ptr)
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
    let mut configured_arg_count = recipe.args.len();
    if let Some(visitor_spec) = &fixture.visitor {
        // Silently dropping the visitor here published a snippet that compiles but omits
        // the one behaviour the fixture exists to demonstrate, under a heading that still
        // promises it — a reader cannot tell that from a language that legitimately needs
        // no visitor. Fail closed instead, matching `php::snippet` and `csharp::snippet`;
        // a deliberate omission belongs in the fixture's `docs.coverage_exceptions`,
        // which records a reader-visible reason. ~keep
        let Some(options_type) =
            options_type.or_else(|| crate::e2e::codegen::recipe::trait_bridge_options_type(config))
        else {
            bail!(
                "Go documentation snippet `{}` needs an options type for its visitor",
                fixture.id
            );
        };
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
        // `replace_go_options` only replaces an existing `nil` slot in place (no arity
        // change) when `args` already ends with one; every other case appends `opts` as
        // a new trailing argument. ~keep
        if !args.ends_with(", nil") {
            configured_arg_count += 1;
        }
        args = replace_go_options(&args);
    }
    if !recipe.extra_args.is_empty() {
        // Bridge/visitor parameters (per `config.trait_bridges`) are real parameters on
        // the extracted Rust function, but the Go binding backend strips them from its
        // emitted signature (see `is_bridge_param` in `gen_bindings::functions`) — so
        // they must not be counted toward the Go-visible arity `extra_args` is clamped
        // against. Falls back to appending every configured `extra_args` verbatim when
        // the call has no resolvable `FunctionDef` (unchanged prior behavior). ~keep
        let bridge_param_names: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|bridge| bridge.param_name.clone())
            .collect();
        let bridge_type_aliases: std::collections::HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|bridge| bridge.type_alias.clone())
            .collect();
        let real_go_param_count = target_function.map(|function| {
            function
                .params
                .iter()
                .filter(|param| !go_is_bridge_param(param, &bridge_param_names, &bridge_type_aliases))
                .count()
        });
        let allowed_extra_args = real_go_param_count
            .map(|limit| limit.saturating_sub(configured_arg_count))
            .unwrap_or(recipe.extra_args.len());
        let extras = recipe.extra_args[..allowed_extra_args.min(recipe.extra_args.len())].join(", ");
        if !extras.is_empty() {
            args = if args.is_empty() {
                extras
            } else {
                format!("{args}, {extras}")
            };
        }
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

    let presentation = crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang);
    Ok(crate::e2e::template_env::render(
        "go/snippet_body.jinja",
        minijinja::context! {
            imports => imports,
            package_decls => package_decls, setup_lines => setup_lines, client_setup => client_setup,
            call_expr => call_expr, result_var => call.result_var, returns_error => returns_error,
            returns_void => call.returns_void,
            expects_error => expects_error,
            error_type => config.error_type_name(),
            import_alias => import_alias,
            presentation => presentation,
        },
    )
    .trim_end()
    .to_string())
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

    fn make_param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

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
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let documented: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let e2e = E2eConfig {
            call: CallConfig {
                function: "process".to_string(),
                module: "github.com/example/library".to_string(),
                result_var: "result".to_string(),
                ..CallConfig::default()
            },
            result_fields: ["summary".to_string(), "items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };

        let body = render_snippet_body(&documented, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(body.contains("result := pkg.Process()"), "{body}");
        assert!(body.contains("fmt.Printf(\"%v\\n\", result.Summary)"), "{body}");
        assert!(body.contains("for _, item := range result.Items {"), "{body}");
        assert!(body.contains("fmt.Printf(\"%+v\\n\", item.Label)"), "{body}");
        assert!(
            !body.contains("fmt.Printf(\"%+v\\n\", result)"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
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
        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

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
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(body.contains("_, err := pkg."), "{body}");
        assert!(body.contains("var typedError pkg.Error"), "{body}");
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

        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

        assert!(!body.contains("\"fmt\""), "{body}");
    }

    #[test]
    fn snippet_separates_package_and_import_declarations() {
        let mut e2e = E2eConfig::default();
        e2e.call.module = "example.com/sample".into();

        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");

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
        let body = render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[], &[])
            .expect("snippet renders");
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
        fixture.input = serde_json::json!({
            "payload": {"kind": "active", "label": "sample", "retry": true, "timeout": 30}
        });
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
                            typed_default: Some(crate::core::ir::DefaultValue::EnumVariant("active".into())),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "label".into(),
                            ty: crate::core::ir::TypeRef::String,
                            typed_default: Some(crate::core::ir::DefaultValue::StringLiteral(String::new())),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "retry".into(),
                            ty: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                            // `needs_omitempty_pointer` (backends::go::gen_bindings::types::helpers) requires
                            // `default.is_some()` — the field's real `#[serde(default)]` attribute, not merely
                            // the container's `impl Default` — before it will treat a non-zero `typed_default`
                            // as pointer-worthy. Without this the field is (correctly) rendered as a required,
                            // non-pointer value and this fixture stops exercising the pointer-cast path it
                            // exists to pin. ~keep
                            default: Some("/* serde(default) */".into()),
                            typed_default: Some(crate::core::ir::DefaultValue::BoolLiteral(true)),
                            ..Default::default()
                        },
                        crate::core::ir::FieldDef {
                            name: "timeout".into(),
                            ty: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
                            default: Some("/* serde(default) */".into()),
                            typed_default: Some(crate::core::ir::DefaultValue::IntLiteral(30)),
                            ..Default::default()
                        },
                    ],
                    has_default: true,
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "SampleConfig".into(),
                    ..TypeDef::default()
                },
            ],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            body.contains(
                "payload := pkg.SampleInput{\n\t\tKind:    ptr(pkg.SampleKind(`active`)),\n\t\tLabel:   `sample`,\n\t\tRetry:   ptr(true),\n\t\tTimeout: ptr(int64(30)),"
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
            &[],
        )
        .expect("snippet renders");

        assert!(rendered.contains("&options"), "{rendered}");
        assert!(rendered.contains("fmt.Printf(\"%+v\\n\", result)"), "{rendered}");
    }

    /// Cluster 1 of the htmd defect: 118 fixtures passed a value where the Go binding
    /// took `*ConversionOptions`. The `options_ptr` config override is hand-authored and
    /// can go stale; when the real `FunctionDef` for the call is available, its
    /// `optional` flag on the options parameter — the same fact
    /// `gen_bindings::functions::gen_function_wrapper` reads to decide `*T` vs `T` — must
    /// win over a stale `options_ptr = false`. ~keep
    #[test]
    fn options_ptr_prefers_the_real_signature_over_a_stale_config_false() {
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
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: false,
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "convert".into(),
            params: vec![
                make_param("html", TypeRef::String, false),
                make_param("options", TypeRef::Named("SampleConfig".into()), true),
            ],
            ..FunctionDef::default()
        }];
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &functions,
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("&options"),
            "the real signature marks the options param `optional`, so the binding backend \
             emits `*SampleConfig` — the snippet must pass `&options` regardless of the \
             config's stale `options_ptr = false`: {rendered}"
        );
    }

    /// The inverse of the above: a stale `options_ptr = true` must not force a pointer
    /// when the real signature takes the options struct by value. ~keep
    #[test]
    fn options_ptr_prefers_the_real_signature_over_a_stale_config_true() {
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
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "convert".into(),
            params: vec![
                make_param("html", TypeRef::String, false),
                make_param("options", TypeRef::Named("SampleConfig".into()), false),
            ],
            ..FunctionDef::default()
        }];
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &functions,
        )
        .expect("snippet renders");

        assert!(
            !rendered.contains("&options"),
            "the real signature's options param is not `optional`, so the binding backend \
             emits a value `SampleConfig` — the snippet must not take its address just \
             because the config's stale `options_ptr = true` says so: {rendered}"
        );
        assert!(rendered.contains("pkg.Convert(options)"), "{rendered}");
    }

    /// Cluster 2 of the htmd defect: 53 fixtures called `htmd.Convert` with more
    /// arguments than the binding accepts. `extra_args` is meant for slots the real
    /// signature actually has (e.g. a visitor-accepting overload); when the resolved
    /// call's `FunctionDef` shows no room left, the configured extras must be dropped
    /// instead of emitted as an argument the binding's `Convert` does not declare. ~keep
    #[test]
    fn extra_args_are_clamped_to_the_real_signatures_remaining_arity() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({ "html": "<p>hi</p>", "options": {} });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![
            crate::e2e::config::ArgMapping {
                name: "html".into(),
                field: "html".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
            crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "options".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            },
        ];
        e2e.call.overrides.insert(
            "go".into(),
            CallOverride {
                module: Some("github.com/example/sample".into()),
                options_type: Some("SampleConfig".into()),
                options_ptr: true,
                extra_args: vec!["nil".into()],
                ..Default::default()
            },
        );
        let functions = [FunctionDef {
            name: "convert".into(),
            params: vec![
                make_param("html", TypeRef::String, false),
                make_param("options", TypeRef::Named("SampleConfig".into()), true),
            ],
            ..FunctionDef::default()
        }];
        let rendered = render_snippet_body(
            &fixture,
            &e2e,
            &ResolvedCrateConfig::default(),
            &[TypeDef {
                name: "SampleConfig".into(),
                ..TypeDef::default()
            }],
            &[],
            &functions,
        )
        .expect("snippet renders");

        assert!(rendered.contains("pkg.Convert("), "{rendered}");
        assert!(
            !rendered.contains(", nil)"),
            "the real `convert` signature has no third parameter, so a configured \
             trailing `extra_args = [\"nil\"]` (sized for a different, visitor-accepting \
             overload) must be dropped rather than emitted as a third positional \
             argument: {rendered}"
        );
    }

    fn visitor_fixture() -> Fixture {
        let mut fixture = fixture();
        fixture.id = "visitor_link_rewrite".into();
        fixture.description = "Visitor rewrites links".into();
        fixture.input = serde_json::json!({ "html": "<a href=\"a\">a</a>" });
        fixture.visitor = serde_json::from_value(serde_json::json!({
            "callbacks": {"visit_link": {"action": "skip"}}
        }))
        .expect("visitor spec");
        fixture
    }

    fn visitor_e2e() -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "convert".into();
        e2e.call.module = "github.com/example/sample".into();
        e2e.call.result_var = "result".into();
        e2e.call.args = vec![crate::e2e::config::ArgMapping {
            name: "html".into(),
            field: "html".into(),
            arg_type: "string".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e
    }

    fn bridge_config(options_type: Option<&str>) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "HtmlVisitor".into(),
                type_alias: Some("VisitorHandle".into()),
                param_name: Some("visitor".into()),
                options_type: options_type.map(str::to_string),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    /// Regression: a visitor fixture with no resolvable options type used to fall through
    /// the `if let` chain, publishing a snippet that compiles but silently omits the
    /// visitor the fixture exists to demonstrate — while the docs page around it still
    /// carries the fixture's visitor title. It must fail closed, matching PHP and C#. ~keep
    #[test]
    fn visitor_without_a_trait_bridge_options_type_fails_instead_of_dropping_the_visitor() {
        let error = render_snippet_body(&visitor_fixture(), &visitor_e2e(), &bridge_config(None), &[], &[], &[])
            .expect_err("a visitor with no options type must not render");

        assert_eq!(
            format!("{error}"),
            "Go documentation snippet `visitor_link_rewrite` needs an options type for its visitor"
        );
    }

    /// Positive control for the above: with the bridge's `options_type` configured, the
    /// ordinary visitor path is unchanged and wires the visitor into the real type. ~keep
    #[test]
    fn visitor_with_a_trait_bridge_options_type_still_wires_the_visitor() {
        let rendered = render_snippet_body(
            &visitor_fixture(),
            &visitor_e2e(),
            &bridge_config(Some("ConversionOptions")),
            &[],
            &[],
            &[],
        )
        .expect("snippet renders");

        assert!(
            rendered.contains("visitor := &testVisitorVisitorLinkRewrite{}"),
            "{rendered}"
        );
        assert!(rendered.contains("opts := &pkg.ConversionOptions{}"), "{rendered}");
        assert!(rendered.contains("opts.Visitor = visitor"), "{rendered}");
        assert!(
            rendered.contains("type testVisitorVisitorLinkRewrite struct{"),
            "{rendered}"
        );
    }
}
