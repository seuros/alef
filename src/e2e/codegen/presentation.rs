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
    /// True when [`Self::expression`] evaluates to an optional/nullable value.
    ///
    /// Distinct from [`Self::optional`], which says the *iterated collection* may be absent and
    /// drives a `?? []`-style guard. This says the value a `show` operation hands to the target
    /// language's print call is itself an optional. Swift needs it because `print`/`debugPrint`
    /// take `Any`, and Swift warns on every implicit optional-to-`Any` coercion — an error under
    /// the `-warnings-as-errors` the snippet validator compiles with. ~keep
    pub(crate) shown_optional: bool,
    /// Per-entry companion to [`Self::fields`], same length and order: whether each iterated
    /// field expression evaluates to an optional. Parallel rather than a struct per field so the
    /// eighteen templates that read `operation.fields` as plain strings keep working unchanged. ~keep
    pub(crate) field_optionals: Vec<bool>,
}

/// True when the value an accessor for `path` yields is optional in the target language.
///
/// An optional link anywhere in the chain makes the whole expression optional — `markdown` being
/// `Option<Markdown>` is what makes `result.markdown()?.content()` a `RustString?` even though
/// `content` itself is not optional — so every prefix is consulted, not just the full path. ~keep
fn path_yields_optional(resolver: &FieldResolver, path: &str) -> bool {
    let segments: Vec<&str> = path.split('.').collect();
    (1..=segments.len()).any(|length| resolver.is_optional(&segments[..length].join(".")))
}

/// `type_defs` feeds the same IR-derived optional-field detection every e2e assertion
/// resolver uses (see `FieldResolver::ir_field_sets`/`with_ir_fields`) so a docs snippet
/// that shows an `Option<T>` field renders the same unwrap/null-check an assertion on
/// that field would, instead of a bare (potentially non-compiling) access. ~keep
pub(crate) fn resolve(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Vec<PresentationOperation> {
    if fixture.docs.is_none() {
        return Vec::new();
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let resolver = build_resolver(e2e_config, call, type_defs);
    resolve_with(fixture, e2e_config, language, &resolver)
}

/// The bare, IR-backed resolver [`resolve`] answers with. Shared with [`apply_derived_shows`] so
/// the paths written into `docs.shows` are decided by exactly the resolver that will later
/// render them, not by a second construction that could drift.
fn build_resolver(
    e2e_config: &E2eConfig,
    call: &crate::core::config::e2e::CallConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> FieldResolver {
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    FieldResolver::new(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
}

/// Write the field paths [`resolve`] would derive from `fixture`'s own assertions into the
/// fixture's `docs.shows`, so that [`Fixture::has_docs_presentation`] reports them.
///
/// ~keep That predicate is the single question the *call emitter* asks about the *snippet's*
/// intent: `rust/test_file/test_function.rs` uses it to decide both whether the call binds a
/// named `result` at all (rather than `let _ =`) and whether a `Result`-returning call is
/// unwrapped before that binding. Before this, it only ever saw hand-authored
/// `shows`/`presentation` blocks, so the operations #199 derives from assertions were invisible
/// to it: 283 generated Rust snippets in one consumer repo bound `let _ = convert(...)` and then
/// printed `result.content` (`E0425`), and any that had bound it would have field-accessed a
/// `Result` (`E0609`). Materializing the derivation into the fixture — rather than teaching the
/// call emitter to re-derive it — is what keeps the two generators reading one fact.
///
/// A fixture that already hand-authors `shows` or `presentation.operations` is left alone: its
/// operations are the authored ones, and `has_docs_presentation` already reports them. Must be
/// called before any caller clears `assertions`, which is where the derivation reads from.
pub(crate) fn apply_derived_shows(
    fixture: &mut Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
) {
    if fixture.docs.is_none() || fixture.has_docs_presentation() {
        return;
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let resolver = build_resolver(e2e_config, call, type_defs);
    let paths: Vec<String> = default_operations_from_assertions(fixture, call, language, &resolver)
        .into_iter()
        .filter_map(|operation| match operation {
            FixtureDocsOperation::Show { path, .. } => Some(path),
            FixtureDocsOperation::Iterate { .. } => None,
        })
        .collect();
    if paths.is_empty() {
        return;
    }
    if let Some(docs) = fixture.docs.as_mut() {
        docs.shows = paths;
    }
}

/// [`resolve`], but against a caller-supplied [`FieldResolver`].
///
/// Two backends cannot use the bare [`FieldResolver::new`] resolver that [`resolve`]
/// builds, because their accessor syntax is decided by a per-type classification the
/// bare resolver does not carry:
///
/// - Swift dispatches property (`result.text`) vs. swift-bridge method (`result.text()`)
///   syntax on a [`SwiftFirstClassMap`]; an empty map classifies every type as opaque,
///   so every accessor would gain a spurious `()`.
/// - PHP dispatches property (`$result->text`) vs. getter (`$result->getText()`) syntax
///   on a [`PhpGetterMap`]; an empty map emits property syntax for the non-scalar fields
///   that ext-php-rs only exposes through a getter.
///
/// [`SwiftFirstClassMap`]: crate::e2e::field_access::SwiftFirstClassMap
/// [`PhpGetterMap`]: crate::e2e::field_access::PhpGetterMap
pub(crate) fn resolve_with(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    resolver: &FieldResolver,
) -> Vec<PresentationOperation> {
    let Some(docs) = fixture.docs.as_ref() else {
        return Vec::new();
    };
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let result_var = call.effective_result_var();
    let result_root = root_variable(language, result_var);
    let operations = docs
        .shows
        .iter()
        .cloned()
        // `docs.shows` is the shorthand form and carries no formatting choice, so it keeps
        // the debug-formatted default; only `presentation.operations` can opt in. ~keep
        .map(|path| FixtureDocsOperation::Show { path, display: false })
        .chain(
            docs.presentation
                .iter()
                .flat_map(|presentation| presentation.operations.iter().cloned()),
        )
        .collect::<Vec<_>>();
    // A fixture-driven docs entry (the common shape: authored once as `assertions`, never
    // hand-annotated with `shows`/`presentation`) has no explicit field list here, but its
    // `assertions` already name the exact result fields it checks -- the same field paths the
    // e2e assertion resolver renders `assert_eq!`/`assertEquals`/etc. against. Reading that
    // existing data instead of leaving the snippet at a bare `print(result)` is what turns
    // "call the function" into "here is how you use what it returns", without a second,
    // independently-derived notion of which fields exist on the result. ~keep
    let operations = if operations.is_empty() {
        default_operations_from_assertions(fixture, call, language, resolver)
    } else {
        operations
    };
    operations
        .iter()
        .map(|operation| match operation {
            FixtureDocsOperation::Show { path, display } => PresentationOperation {
                kind: "show",
                expression: resolver.accessor(path, language, &result_root),
                item: String::new(),
                fields: Vec::new(),
                optional: false,
                display: *display,
                destructure_source: String::new(),
                destructure_item: String::new(),
                shown_optional: path_yields_optional(resolver, path),
                field_optionals: Vec::new(),
            },
            FixtureDocsOperation::Iterate {
                path,
                item,
                fields,
                display,
                optional,
            } => {
                let (destructure_source, destructure_item, expression) =
                    typescript_first_item(path, language, resolver, &result_root);
                let item_root = root_variable(language, item);
                PresentationOperation {
                    kind: "iterate",
                    expression,
                    item: item.clone(),
                    fields: fields
                        .iter()
                        .map(|field| resolver.accessor(field, language, &item_root))
                        .collect(),
                    // A fixture's own `optional` flag is authored by hand and can
                    // drift from the field-optionality data already known to the
                    // resolver (`fields_optional` in the e2e config). When the
                    // resolver knows the iterated path is optional but the fixture
                    // wasn't updated to say so, trusting only `*optional` emits a
                    // bare `for (const x of first?.optionalField)` with no `?? []`
                    // guard -- a TS18048 in strict mode. OR the two signals so a
                    // stale fixture flag can't regress a snippet that alef already
                    // has the type information to render safely.
                    optional: *optional || resolver.is_optional(path),
                    display: *display,
                    destructure_source,
                    destructure_item,
                    shown_optional: false,
                    field_optionals: fields
                        .iter()
                        .map(|field| path_yields_optional(resolver, field))
                        .collect(),
                }
            }
        })
        .collect()
}

/// Default field-access operations for a docs-tagged fixture whose `docs.shows` and
/// `docs.presentation.operations` are both empty.
///
/// Every generated assertion already anchors on `Assertion::field`, so the field paths a
/// fixture cares about are known even when nobody hand-authored a `shows` list for the docs
/// snippet. This derives one `show` per distinct field, in first-appearance order, from
/// exactly that same data -- deliberately not re-deriving field names from the IR or the
/// input shape, which would let this and the assertion resolver disagree about what fields a
/// result has. Assertions with no `field` (method-result checks, `error` assertions) name
/// nothing to show and are skipped; a void call has no result to access at all. ~keep
///
/// A derived path is only shown when the assertion renderer would itself have rendered a member
/// access on the result for it. `Assertion::field` is not a promise that the name is a member of
/// the return type — three whole classes of assertion name something else, and 0.67.2 emitted a
/// non-compiling accessor for every one of them:
///
/// * an **error-path fixture**. Every backend's error block renders the must-fail check and
///   returns without visiting another assertion (that is the entire premise of
///   [`error_path_assertions`]), so `error.status_code` is a claim about the raised error, never
///   about a result — and on the success path there is no result to show anyway.
/// * a **non-struct result**. When `result_is_simple`/`result_is_bytes` holds for this language,
///   the field is a pseudo-field naming the buffer or scalar itself, exactly as
///   `java/assertions.rs`'s byte-buffer arm documents; the snippet falls back to showing the
///   whole result.
/// * a **name the availability oracle does not recognize**. Both halves are needed:
///   [`FieldResolver::is_valid_for_result`] rejects what the oracle positively excludes, and
///   [`FieldResolver::result_field_oracle_knows`] additionally rejects what it has simply never
///   heard of — an assertion *grouping* like `rate_limit.` or a streaming pseudo-field is not a
///   member path, and defaulting an unrecognized name to "valid" is right for an authored
///   assertion but wrong for an inferred accessor. See that method for the asymmetry.
///
/// Rejection falls back to no operation, i.e. the pre-#199 whole-result display — never to a
/// guess. ~keep
///
/// [`error_path_assertions`]: crate::e2e::codegen::error_path_assertions
fn default_operations_from_assertions(
    fixture: &Fixture,
    call: &crate::core::config::e2e::CallConfig,
    language: &str,
    resolver: &FieldResolver,
) -> Vec<FixtureDocsOperation> {
    if call.returns_void
        || call.effective_result_is_simple(language)
        || call.effective_result_is_bytes(language)
        || fixture.assertions.iter().any(|a| a.assertion_type == "error")
    {
        return Vec::new();
    }
    let mut seen_fields = Vec::new();
    fixture
        .assertions
        .iter()
        .filter_map(|assertion| assertion.field.as_deref())
        .filter(|field| shows_on_result(field, resolver))
        .filter(|field| {
            let is_new = !seen_fields.contains(field);
            if is_new {
                seen_fields.push(*field);
            }
            is_new
        })
        .map(|field| FixtureDocsOperation::Show {
            path: field.to_string(),
            display: false,
        })
        .collect()
}

/// Whether a derived field path names a member of the call's result, per the oracles the
/// assertion renderers already consult. See [`default_operations_from_assertions`].
fn shows_on_result(field: &str, resolver: &FieldResolver) -> bool {
    !field.is_empty()
        && !crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(field)
        && resolver.is_valid_for_result(field)
        && resolver.result_field_oracle_knows(field) != Some(false)
}

/// The root variable an accessor chain is anchored on, spelled the way the target
/// language spells a variable reference.
///
/// PHP is the only backend whose variables carry a sigil, and the sigil has to be part
/// of the root handed to `FieldResolver::accessor` rather than prepended in the
/// template: `render_php` wraps a trailing `.length` segment as `count(<chain>)`, so a
/// template-side `$` would land outside the call (`$count(...)`) instead of on the
/// variable. Matches `php::assertions`, which passes `format!("${result_var}")`. ~keep
fn root_variable(language: &str, name: &str) -> String {
    if language == "php" {
        format!("${name}")
    } else {
        name.to_string()
    }
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
        return (format!("{source} ?? []"), "first".into(), format!("first?.{tail}"));
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
                input: None,
                shows: Vec::new(),
                error: None,
                presentation: Some(FixtureDocsPresentation {
                    call: None,
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
                    files: Vec::new(),
                    operations: vec![FixtureDocsOperation::Iterate {
                        path: "items".into(),
                        item: "item".into(),
                        fields: vec!["text".into(), "metadata.heading".into()],
                        display: true,
                        optional: true,
                    }],
                }),
                client: None,
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
    fn docs_call_fixture_removes_mock_harness_and_uses_an_illustrative_url() {
        let mut fixture = fixture();
        fixture
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .input = None;
        fixture.input = serde_json::json!({
            "mock_responses": [{"path": "/guide.txt", "status_code": 200}],
            "extract_input": {"kind": "uri", "uri": "$mock_url/guide.txt"}
        });
        fixture.mock_response = Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: None,
            stream_chunks: None,
            headers: Default::default(),
        });

        let docs_fixture = fixture.docs_call_fixture();

        assert!(docs_fixture.mock_response.is_none());
        assert!(docs_fixture.input.get("mock_responses").is_none());
        assert_eq!(
            docs_fixture
                .input
                .pointer("/extract_input/uri")
                .and_then(serde_json::Value::as_str),
            Some("https://example.com/guide.txt")
        );
        assert!(!docs_fixture.needs_mock_server());
    }

    #[test]
    fn show_display_flag_selects_the_human_readable_rust_formatter() {
        let mut display_fixture = fixture();
        display_fixture
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .operations = vec![FixtureDocsOperation::Show {
            path: "text".into(),
            display: true,
        }];
        let mut debug_fixture = fixture();
        debug_fixture
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .operations = vec![FixtureDocsOperation::Show {
            path: "text".into(),
            display: false,
        }];
        let config = config();

        let render = |operations| {
            crate::e2e::template_env::render(
                "rust/snippet_body.rs.jinja",
                minijinja::context! { imports => Vec::<String>::new(), body => vec!["let result = process();"],
                is_async => false, presentation => operations },
            )
        };
        let displayed = render(resolve(&display_fixture, &config, "rust", &[]));
        let debugged = render(resolve(&debug_fixture, &config, "rust", &[]));

        assert!(displayed.contains("println!(\"{}\", result.text);"), "{displayed}");
        assert!(debugged.contains("println!(\"{:?}\", result.text);"), "{debugged}");
    }

    #[test]
    fn presentation_templates_emit_idiomatic_python_rust_and_typescript() {
        let fixture = fixture();
        let config = config();
        let python = resolve(&fixture, &config, "python", &[]);
        let rust = resolve(&fixture, &config, "rust", &[]);
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
        let typescript = resolve(&typescript_fixture, &config, "node", &[]);

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
            typescript_output.contains("const [first] = result.results ?? [];"),
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

    /// A fixture's own `optional: false` on an `Iterate` operation must not
    /// override field-optionality the resolver already knows about (from the
    /// e2e config's `fields_optional`). `config_element_types.json` hit this:
    /// `results[0].elements` is a genuinely optional field (registered in
    /// `fields_optional`), but the fixture's `Iterate` operation didn't set
    /// `"optional": true`, so the generated node/wasm snippet rendered
    /// `for (const element of first?.elements)` with no `?? []` guard --
    /// `first?.elements` is `Element[] | undefined`, and iterating it directly
    /// is a `tsc` TS18048 in strict mode.
    #[test]
    fn resolve_iterate_treats_path_optional_when_fixture_flag_is_stale() {
        let mut stale_fixture = fixture();
        stale_fixture
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .operations = vec![FixtureDocsOperation::Iterate {
            path: "results[0].elements".into(),
            item: "element".into(),
            fields: vec!["element_type".into()],
            display: true,
            optional: false,
        }];
        let mut stale_config = config();
        stale_config.fields_optional = ["results[0].elements".to_string()].into_iter().collect();

        let operations = resolve(&stale_fixture, &stale_config, "node", &[]);
        let iterate = operations.first().expect("one iterate operation");
        assert!(
            iterate.optional,
            "resolver-known optionality for 'results[0].elements' must win over the fixture's stale `optional: false`"
        );

        let typescript_output = crate::e2e::template_env::render(
            "typescript/snippet_body.jinja",
            minijinja::context! { imports => vec!["process"], module => "@example/library",
            setup_lines => Vec::<String>::new(), client_setup => "", call_expr => "process()",
            result_var => "result", is_async => false, expects_error => false,
            presentation => operations },
        );
        assert!(
            typescript_output.contains("for (const element of first?.elements ?? [])"),
            "{typescript_output}"
        );
    }

    /// A docs snippet that shows a field reached through an `Option<T>` in a non-leaf
    /// position must unwrap, even when the consumer's `alef.toml` never lists that field
    /// under `fields_optional` -- the IR alone (`FieldDef.optional`) must be enough. This
    /// is the snippet-surface half of the same bug the e2e assertion resolver had: passing
    /// real `type_defs` changes the rendered accessor, and `&[]` (no IR) reproduces the old
    /// (broken) behavior -- proving the merge in `resolve` actually takes effect rather
    /// than every new-parameter call site silently passing an empty set. ~keep
    #[test]
    fn resolve_show_unwraps_ir_only_optional_field_in_non_leaf_position() {
        use crate::core::ir::{FieldDef, TypeDef};

        let mut show_fixture = fixture();
        show_fixture
            .docs
            .as_mut()
            .and_then(|docs| docs.presentation.as_mut())
            .expect("presentation")
            .operations = vec![FixtureDocsOperation::Show {
            path: "data.kind".into(),
            display: false,
        }];
        // No `fields_optional` entry for `data` anywhere in this config -- optionality
        // must come entirely from the IR passed to `resolve`.
        let config = config();
        assert!(!config.fields_optional.contains("data"));

        let process_result = TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![FieldDef {
                name: "data".to_string(),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        };

        let without_ir = resolve(&show_fixture, &config, "rust", &[]);
        let with_ir = resolve(&show_fixture, &config, "rust", &[process_result]);

        assert_eq!(
            without_ir[0].expression, "result.data.kind",
            "with no IR in scope, resolve falls back to the pre-fix (non-compiling) accessor"
        );
        assert_eq!(
            with_ir[0].expression, "result.data.as_ref().unwrap().kind",
            "with IR in scope, resolve must unwrap the Option before the nested field access"
        );
    }

    /// The shape every fixture-driven (non-hand-authored) docs fixture takes: `docs` is
    /// present so the fixture DOES get a snippet, but nobody hand-annotated `shows` or
    /// `presentation` -- the only field knowledge lives in `assertions`. Before this fell
    /// back to reading `assertions`, `resolve` returned an empty operations list here and
    /// every generated snippet in every language bottomed out at a bare
    /// `print(result)`/`println!("{:?}", result)`, never showing how to consume the return
    /// value. Two assertions on the same field (`equals` and `not_empty`, both on
    /// `"content"`) must collapse to one `show`, not print the field twice.
    #[test]
    fn resolve_derives_show_operations_from_assertion_fields_when_docs_names_none() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "smoke_simple_paragraph",
            "description": "Simple paragraph converts correctly",
            "input": {"html": "<p>Hello World</p>"},
            "assertions": [
                {"type": "equals", "field": "content", "value": "Hello World\n"},
                {"type": "not_empty", "field": "content"}
            ],
            "docs": {"topic": "smoke", "stem": "smoke_simple_paragraph"}
        }))
        .expect("fixture must parse");
        let config = E2eConfig {
            call: CallConfig {
                function: "convert".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            ..E2eConfig::default()
        };

        let python = resolve(&fixture, &config, "python", &[]);
        assert_eq!(python.len(), 1, "the duplicate 'content' field must not be shown twice");
        assert_eq!(python[0].kind, "show");
        assert_eq!(python[0].expression, "result.content");

        let rust = resolve(&fixture, &config, "rust", &[]);
        assert_eq!(rust[0].expression, "result.content");
    }

    /// An `error`-typed assertion names no `field` and must not be mistaken for one -- it
    /// documents a failure mode, not a field to print on the success path.
    #[test]
    fn resolve_ignores_assertions_with_no_field_when_deriving_show_operations() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "auth_error",
            "description": "Authentication failure",
            "input": {"token": "bad"},
            "assertions": [{"type": "error"}],
            "docs": {"topic": "errors", "stem": "auth_error"}
        }))
        .expect("fixture must parse");
        let config = config();

        assert!(resolve(&fixture, &config, "python", &[]).is_empty());
    }

    /// A void call has no result to access; even a fixture whose assertions happen to name a
    /// field (e.g. a side-effect check) must not gain a fabricated `print(result.<field>)`.
    #[test]
    fn resolve_derives_no_show_operations_for_a_void_returning_call() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "configure_logging",
            "description": "Configure logging",
            "input": {"level": "debug"},
            "assertions": [{"type": "equals", "field": "level", "value": "debug"}],
            "docs": {"topic": "configuration", "stem": "configure_logging"}
        }))
        .expect("fixture must parse");
        let mut config = config();
        config.call.returns_void = true;

        assert!(resolve(&fixture, &config, "python", &[]).is_empty());
    }
}

#[cfg(test)]
#[path = "presentation/derived_show_resolution_tests.rs"]
mod derived_show_resolution_tests;
