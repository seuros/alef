use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureDocsOperation};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct PresentationOperation {
    pub(crate) kind: &'static str,
    pub(crate) expression: String,
    pub(crate) item: String,
    pub(crate) fields: Vec<String>,
    pub(crate) optional: bool,
    pub(crate) display: bool,
    pub(crate) destructure_source: String,
    pub(crate) destructure_item: String,
}

pub(crate) fn resolve(fixture: &Fixture, e2e_config: &E2eConfig, language: &str) -> Vec<PresentationOperation> {
    let Some(presentation) = fixture.docs.as_ref().and_then(|docs| docs.presentation.as_ref()) else {
        return Vec::new();
    };
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let resolver = FieldResolver::new(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
    );
    presentation
        .operations
        .iter()
        .map(|operation| match operation {
            FixtureDocsOperation::Show { path } => PresentationOperation {
                kind: "show",
                expression: resolver.accessor(path, language, &call.result_var),
                item: String::new(),
                fields: Vec::new(),
                optional: false,
                display: false,
                destructure_source: String::new(),
                destructure_item: String::new(),
            },
            FixtureDocsOperation::Iterate {
                path,
                item,
                fields,
                display,
                optional,
            } => {
                let (destructure_source, destructure_item, expression) =
                    typescript_first_item(path, language, &resolver, &call.result_var);
                PresentationOperation {
                    kind: "iterate",
                    expression,
                    item: item.clone(),
                    fields: fields
                        .iter()
                        .map(|field| resolver.accessor(field, language, item))
                        .collect(),
                    optional: *optional,
                    display: *display,
                    destructure_source,
                    destructure_item,
                }
            }
        })
        .collect()
}

fn typescript_first_item(
    path: &str,
    language: &str,
    resolver: &FieldResolver,
    result_var: &str,
) -> (String, String, String) {
    if matches!(language, "node" | "wasm")
        && let Some((source, tail)) = path.split_once("[0].")
    {
        let source = resolver.accessor(source, language, result_var);
        return (source, "first".into(), format!("first?.{tail}"));
    }
    (
        String::new(),
        String::new(),
        resolver.accessor(path, language, result_var),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{ArgMapping, CallConfig};
    use crate::e2e::fixture::{FixtureDocs, FixtureDocsPresentation, SideEffectClass};
    use std::collections::BTreeMap;

    fn fixture() -> Fixture {
        Fixture {
            id: "present_items".into(),
            description: "Present returned items".into(),
            input: serde_json::json!({"old_source": "test.txt"}),
            docs: Some(FixtureDocs {
                topic: "configuration".into(),
                stem: None,
                paths: BTreeMap::new(),
                title: None,
                description: None,
                presentation: Some(FixtureDocsPresentation {
                    input: Some(serde_json::json!({"source": "guide.txt"})),
                    args: Some(vec![ArgMapping {
                        name: "source".into(),
                        field: "source".into(),
                        arg_type: "string".into(),
                        optional: false,
                        owned: false,
                        element_type: None,
                        go_type: None,
                        vec_inner_is_ref: false,
                        trait_name: None,
                    }]),
                    operations: vec![FixtureDocsOperation::Iterate {
                        path: "items".into(),
                        item: "item".into(),
                        fields: vec!["text".into(), "metadata.heading".into()],
                        display: true,
                        optional: true,
                    }],
                }),
                side_effects: SideEffectClass::Safe,
                coverage_exceptions: BTreeMap::new(),
            }),
            ..Fixture::default()
        }
    }

    fn config() -> E2eConfig {
        E2eConfig {
            call: CallConfig {
                function: "process".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            fields_optional: ["items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        }
    }

    #[test]
    fn docs_call_overrides_reuse_typed_fixture_arguments() {
        let fixture = fixture().docs_call_fixture();
        assert_eq!(fixture.input, serde_json::json!({"source": "guide.txt"}));
        assert_eq!(fixture.args[0].arg_type, "string");
        assert_eq!(fixture.args[0].field, "source");
    }

    #[test]
    fn presentation_templates_emit_idiomatic_python_rust_and_typescript() {
        let fixture = fixture();
        let config = config();
        let python = resolve(&fixture, &config, "python");
        let rust = resolve(&fixture, &config, "rust");
        let mut typescript_fixture = fixture.clone();
        typescript_fixture
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .operations = vec![FixtureDocsOperation::Iterate {
            path: "results[0].chunks".into(),
            item: "chunk".into(),
            fields: vec!["content".into()],
            display: true,
            optional: true,
        }];
        let typescript = resolve(&typescript_fixture, &config, "node");

        let python_output = crate::e2e::template_env::render(
            "python/snippet_body.py.jinja",
            minijinja::context! { imports => Vec::<String>::new(), body => vec!["result = process()"],
            is_async => false, presentation => python },
        );
        let rust_output = crate::e2e::template_env::render(
            "rust/snippet_body.rs.jinja",
            minijinja::context! { imports => Vec::<String>::new(), body => vec!["let result = process();"],
            is_async => false, presentation => rust },
        );
        let typescript_output = crate::e2e::template_env::render(
            "typescript/snippet_body.jinja",
            minijinja::context! { imports => vec!["process"], module => "@example/library",
            setup_lines => Vec::<String>::new(), client_setup => "", call_expr => "process()",
            result_var => "result", is_async => false, expects_error => false,
            presentation => typescript },
        );

        assert!(
            python_output.contains("for item in result.items or []:"),
            "{python_output}"
        );
        assert!(
            python_output.contains("print(item.metadata.heading)"),
            "{python_output}"
        );
        assert!(
            rust_output.contains("for item in result.items.iter().flatten()"),
            "{rust_output}"
        );
        assert!(
            rust_output.contains("println!(\"{}\", item.metadata.heading);"),
            "{rust_output}"
        );
        assert!(
            typescript_output.contains("const [first] = result.results;"),
            "{typescript_output}"
        );
        assert!(
            typescript_output.contains("for (const chunk of first?.chunks ?? [])"),
            "{typescript_output}"
        );
        assert!(
            typescript_output.contains("console.log(chunk.content);"),
            "{typescript_output}"
        );
    }
}
