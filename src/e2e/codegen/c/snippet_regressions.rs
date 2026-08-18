use super::*;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

fn json_arg(name: &str, field: &str, element_type: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some(element_type.into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

pub(super) fn compile_snippet(rendered: &str, header_name: &str, header: &str) {
    let Some(compiler) = ["cc", "clang", "gcc"]
        .into_iter()
        .find(|candidate| which::which(candidate).is_ok())
    else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary C snippet directory");
    std::fs::write(directory.path().join(header_name), header).expect("write neutral C header");
    let source = directory.path().join("snippet.c");
    std::fs::write(&source, rendered).expect("write generated C snippet");
    let output = std::process::Command::new(compiler)
        .args(["-std=c11", "-fsyntax-only", "-Wall", "-Werror", "-I"])
        .arg(directory.path())
        .arg(&source)
        .output()
        .expect("run C compiler");
    assert!(
        output.status.success(),
        "generated C snippet failed to compile:\n{}\n{rendered}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn whole_input_typed_file_snippet_constructs_and_owns_the_public_handle() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "document_input",
        "description": "Process a document",
        "input": {"extract_input": {"kind": "bytes", "content": [1, 2, 3]}},
        "docs": {"topic": "guides", "presentation": {
            "files": [{"field": "/extract_input/content", "path": "document.bin"}]
        }}
    }))
    .expect("fixture");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    e2e.call.args.push(json_arg("input", "input", "DocumentInput"));
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            result_type: Some("DocumentResult".into()),
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("typed file snippet renders");

    assert!(
        rendered.contains("sample_document_input_from_json(input_json_0)"),
        "{rendered}"
    );
    assert!(rendered.contains("sample_process(input_handle)"), "{rendered}");
    assert!(
        rendered.contains("sample_document_input_free(input_handle)"),
        "{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_document_input_from_json(const char *json);\n",
            "void sample_document_input_free(SAMPLEAlefHandle input);\n",
            "SAMPLEAlefHandle sample_process(SAMPLEAlefHandle input);\n",
            "void sample_document_result_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

#[test]
fn multiple_typed_args_and_ir_return_shape_match_the_public_abi() {
    let fixture = Fixture {
        id: "convert".into(),
        description: "Convert a source".into(),
        input: serde_json::json!({"source": {"text": "hello"}, "settings": {"mode": "fast"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    e2e.call.options_type = Some("ObsoleteOptions".into());
    e2e.call.args = vec![
        json_arg("source", "input.source", "SourceInput"),
        crate::e2e::config::ArgMapping {
            element_type: None,
            ..json_arg("settings", "input.settings", "ConvertSettings")
        },
    ];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "convert".into(),
        params: vec![
            ParamDef {
                name: "source".into(),
                ty: TypeRef::Named("SourceInput".into()),
                ..ParamDef::default()
            },
            ParamDef {
                name: "settings".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ConvertSettings".into()))),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("ConversionReceipt".into()),
        ..FunctionDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &functions).expect("multi-arg snippet renders");

    assert!(rendered.contains("sample_source_input_from_json"), "{rendered}");
    assert!(rendered.contains("sample_convert_settings_from_json"), "{rendered}");
    assert!(
        rendered.contains("sample_convert(source_handle, settings_handle)"),
        "{rendered}"
    );
    assert!(rendered.contains("SAMPLEAlefHandle result"), "{rendered}");
    assert!(!rendered.contains("ObsoleteOptions"), "{rendered}");
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_source_input_from_json(const char *json);\n",
            "SAMPLEAlefHandle sample_convert_settings_from_json(const char *json);\n",
            "void sample_source_input_free(SAMPLEAlefHandle value);\n",
            "void sample_convert_settings_free(SAMPLEAlefHandle value);\n",
            "SAMPLEAlefHandle sample_convert(SAMPLEAlefHandle source, SAMPLEAlefHandle settings);\n",
            "void sample_conversion_receipt_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

fn optional_arg(name: &str, field: &str, arg_type: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: arg_type.into(),
        optional: true,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// Regression for the "legacy" (non-client-factory) opaque-handle call path
/// (e.g. `htm_convert(html, options)`): an absent optional `json_object` arg
/// must render as the scalar `AlefHandle` "none" sentinel `0`, not the pointer
/// sentinel `NULL` — the C ABI maps every `TypeRef::Named` param to
/// `typedef uint64_t {PREFIX}AlefHandle`, so `NULL` is an incompatible
/// pointer-to-integer conversion (`clang -Wint-conversion`) at the call site.
#[test]
fn absent_optional_json_object_arg_uses_the_handle_sentinel_not_null() {
    let fixture = Fixture {
        id: "convert_no_options".into(),
        description: "Convert without options".into(),
        input: serde_json::json!({"html": "<p>hi</p>"}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    e2e.call.args = vec![
        optional_arg("html", "input.html", "string"),
        crate::e2e::config::ArgMapping {
            optional: true,
            ..json_arg("options", "input.options", "ConvertOptions")
        },
    ];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "convert".into(),
        params: vec![
            ParamDef {
                name: "html".into(),
                ty: TypeRef::String,
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ConvertOptions".into()))),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("ConversionResult".into()),
        ..FunctionDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &functions).expect("optional-arg snippet renders");

    // The absent `options` handle must be `0`, never `NULL`.
    assert!(
        rendered.contains("sample_convert(\"<p>hi</p>\", 0)"),
        "expected the absent optional handle arg to render as `0`:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_convert(\"<p>hi</p>\", NULL)"),
        "absent optional handle arg must not use the pointer sentinel `NULL`:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_convert(const char *html, SAMPLEAlefHandle options);\n",
            "void sample_conversion_result_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

/// Companion regression: an absent optional `string` arg must keep rendering
/// as `NULL` — proves the fix is type-aware rather than a blanket
/// `NULL` → `0` replacement, which would silently break real `const char *`
/// pointer parameters.
#[test]
fn absent_optional_string_arg_still_uses_null_pointer_sentinel() {
    let fixture = Fixture {
        id: "convert_no_hint".into(),
        description: "Convert without a format hint".into(),
        input: serde_json::json!({"html": "<p>hi</p>"}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    e2e.call.args = vec![
        optional_arg("html", "input.html", "string"),
        optional_arg("format_hint", "input.format_hint", "string"),
    ];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "convert".into(),
        params: vec![
            ParamDef {
                name: "html".into(),
                ty: TypeRef::String,
                ..ParamDef::default()
            },
            ParamDef {
                name: "format_hint".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("ConversionResult".into()),
        ..FunctionDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &functions).expect("optional-string snippet renders");

    assert!(
        rendered.contains("sample_convert(\"<p>hi</p>\", NULL)"),
        "expected the absent optional `const char *` arg to keep the `NULL` sentinel:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_convert(\"<p>hi</p>\", 0)"),
        "a real pointer arg must never be replaced with the handle sentinel `0`:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_convert(const char *html, const char *format_hint);\n",
            "void sample_conversion_result_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

#[test]
fn list_return_uses_owned_json_string_abi() {
    let fixture = Fixture {
        id: "list_items".into(),
        description: "List items".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "sample_list_items".into();
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "sample_list_items".into(),
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))),
        ..FunctionDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &functions).expect("list snippet renders");

    assert!(rendered.contains("char* result = sample_list_items();"), "{rendered}");
    assert!(rendered.contains("sample_free_string(result);"), "{rendered}");
    assert!(!rendered.contains("SAMPLEList"), "{rendered}");
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "char *sample_list_items(void);\n",
            "void sample_free_string(char *value);\n"
        ),
    );
}

/// Regression for the emitter contract that every symbol a C snippet writes must
/// resolve inside the emitted translation unit.
///
/// A fixture carrying `[env] api_key_var` and no mock server renders an
/// `ALEF_TEST_SKIP(...)` guard. That macro is declared only by the generated e2e
/// *runner* header, which a standalone documentation snippet never includes, so
/// the snippet has to carry its own definition — without one the emitted unit
/// fails to compile with an implicit-function-declaration error.
///
/// The fixture is deliberately identical to `list_return_uses_owned_json_string_abi`
/// apart from the `env` block, so the env guard is the only difference between a
/// snippet that compiles and one that does not.
#[test]
fn env_gated_snippet_defines_the_skip_macro_it_uses() {
    let fixture = Fixture {
        id: "smoke_list_items".into(),
        description: "List items against the real API".into(),
        env: Some(crate::e2e::fixture::FixtureEnv {
            api_key_var: Some("SAMPLE_API_KEY".into()),
        }),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "sample_list_items".into();
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "sample_list_items".into(),
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))),
        ..FunctionDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &functions).expect("env-gated snippet renders");

    let macro_use = "ALEF_TEST_SKIP(\"SAMPLE_API_KEY not set\")";
    let macro_definition = "#define ALEF_TEST_SKIP(reason)";
    assert!(
        rendered.contains(macro_use),
        "expected the env guard to be emitted:\n{rendered}"
    );
    assert!(
        rendered.contains(macro_definition),
        "snippet references ALEF_TEST_SKIP without defining it:\n{rendered}"
    );
    let define_position = rendered.find(macro_definition).expect("macro definition");
    let use_position = rendered.find(macro_use).expect("macro use");
    assert!(
        define_position < use_position,
        "the macro definition must precede its use:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "char *sample_list_items(void);\n",
            "void sample_free_string(char *value);\n"
        ),
    );
}

#[test]
fn client_snippet_keeps_adapter_identity_bare() {
    let fixture = Fixture {
        id: "convert".into(),
        description: "Convert a document".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![crate::core::config::AdapterConfig {
            name: "convert".into(),
            pattern: crate::core::config::AdapterPattern::AsyncMethod,
            core_path: "sample::convert".into(),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some("DefaultClient".into()),
            item_type: None,
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }],
        ..ResolvedCrateConfig::default()
    };

    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("client snippet renders");

    assert!(rendered.contains("sample_default_client_convert(client)"), "{rendered}");
    assert!(!rendered.contains("sample_default_client_sample_convert"), "{rendered}");
    assert!(!rendered.contains("skipped:"), "{rendered}");
}

fn mock_backed_fixture(id: &str) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "description": "Backed by the e2e mock server",
        "input": {},
        "mock_response": {"status": 200, "body": {}}
    }))
    .expect("fixture")
}

fn empty_field_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn streaming_client_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![
            serde_json::from_value(serde_json::json!({
                "name": "chat_stream",
                "pattern": "streaming",
                "core_path": "sample::chat_stream",
                "owner_type": "DefaultClient",
                "item_type": "ChatChunk",
                "request_type": "sample::ChatRequest"
            }))
            .expect("streaming adapter config"),
        ],
        ..ResolvedCrateConfig::default()
    }
}

fn streaming_client_e2e() -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat_stream".into();
    e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    e2e
}

fn bytes_client_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![
            serde_json::from_value(serde_json::json!({
                "name": "speech",
                "pattern": "async_method",
                "core_path": "sample::speech",
                "owner_type": "DefaultClient"
            }))
            .expect("bytes adapter config"),
        ],
        ..ResolvedCrateConfig::default()
    }
}

fn bytes_client_e2e() -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "speech".into();
    e2e.call.result_is_bytes = true;
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    e2e
}

fn assert_no_mock_harness(rendered: &str, fixture_id: &str) {
    assert!(
        !rendered.contains("MOCK_SERVER"),
        "mock-server env var leaked:\n{rendered}"
    );
    assert!(
        !rendered.contains(&format!("/fixtures/{fixture_id}")),
        "mock-server fixture route leaked:\n{rendered}"
    );
    assert!(
        !rendered.contains("\"test-key\""),
        "literal credential leaked:\n{rendered}"
    );
}

/// The streaming and byte-buffer emitters each computed `has_mock` from
/// `Fixture::needs_mock_server()` alone, unlike `test_function::render_test_function`,
/// which ANDs it with `!documentation_snippet`. Their `else` arms then hardcoded the
/// harness credential `"test-key"` — which the central
/// `snippets::reject_mock_harness_scaffolding` guard does *not* list as a marker — so a
/// streaming or bytes fixture published a snippet telling the reader to authenticate
/// with the e2e suite's fake key. ~keep
#[test]
fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
    let streaming = mock_backed_fixture("chat_stream_basic");
    let rendered = render_c_snippet(
        &streaming,
        &streaming_client_e2e(),
        &streaming_client_config(),
        &[],
        &[],
    )
    .expect("streaming snippet renders");

    assert_no_mock_harness(&rendered, "chat_stream_basic");
    assert!(
        rendered.contains("const char* api_key = getenv(\"API_KEY\");"),
        "credential is not read from the environment:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_create_client(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL)"),
        "streaming client is not constructed the way a reader would:\n{rendered}"
    );

    let bytes = mock_backed_fixture("speech_basic");
    let rendered =
        render_c_snippet(&bytes, &bytes_client_e2e(), &bytes_client_config(), &[], &[]).expect("bytes snippet renders");

    assert_no_mock_harness(&rendered, "speech_basic");
    assert!(
        rendered.contains("const char* api_key = getenv(\"API_KEY\");"),
        "credential is not read from the environment:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_create_client(api_key, NULL, (uint64_t)-1, (uint32_t)-1, NULL)"),
        "bytes client is not constructed the way a reader would:\n{rendered}"
    );
}

/// Companion pin for the fix above: the same two fixtures rendered in e2e test mode
/// (`documentation_snippet == false`) must keep every byte of the mock-server wiring the
/// suites actually run against. The naive fix — dropping the mock branch outright —
/// passes the snippet assertions and silently unplugs the C e2e suite. ~keep
#[test]
fn e2e_test_functions_still_point_the_client_at_the_mock_server() {
    let resolver = empty_field_resolver();

    let mut streaming_out = String::new();
    render_test_function(
        &mut streaming_out,
        &mock_backed_fixture("chat_stream_basic"),
        "sample",
        "chat_stream",
        "result",
        &[],
        &resolver,
        &HashMap::new(),
        &HashSet::new(),
        &ResultTypeName::Resolved("ChatChunk".into()),
        "",
        Some("create_client"),
        None,
        None,
        None,
        false,
        false,
        Some(true),
        &[],
        &streaming_client_config(),
        &[],
        false,
        &FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::Global,
        },
    )
    .expect("streaming test function renders");

    assert!(
        streaming_out.contains("const char* mock_base = getenv(\"MOCK_SERVER_URL\");"),
        "{streaming_out}"
    );
    assert!(
        streaming_out.contains("\"%s/fixtures/chat_stream_basic\""),
        "{streaming_out}"
    );
    assert!(
        streaming_out.contains("sample_create_client(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"),
        "{streaming_out}"
    );

    let mut bytes_out = String::new();
    render_test_function(
        &mut bytes_out,
        &mock_backed_fixture("speech_basic"),
        "sample",
        "speech",
        "result",
        &[],
        &resolver,
        &HashMap::new(),
        &HashSet::new(),
        &ResultTypeName::Resolved("SpeechResponse".into()),
        "",
        Some("create_client"),
        None,
        None,
        None,
        false,
        true,
        None,
        &[],
        &bytes_client_config(),
        &[],
        false,
        &FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::Global,
        },
    )
    .expect("bytes test function renders");

    assert!(
        bytes_out.contains("const char* mock_base = getenv(\"MOCK_SERVER_URL\");"),
        "{bytes_out}"
    );
    assert!(bytes_out.contains("\"%s/fixtures/speech_basic\""), "{bytes_out}");
    assert!(
        bytes_out.contains("sample_create_client(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"),
        "{bytes_out}"
    );
}

#[test]
fn client_snippet_without_owner_metadata_is_rejected() {
    let fixture = Fixture {
        id: "convert".into(),
        description: "Convert a document".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let error = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect_err("missing owner must fail");

    assert!(
        error.to_string().contains("could not resolve the client owner type"),
        "{error:#}"
    );
}

fn required_arg(name: &str, field: &str, arg_type: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: arg_type.into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn handle_param_config_type() -> crate::core::ir::TypeDef {
    crate::core::ir::TypeDef {
        name: "SampleConfig".into(),
        rust_path: "samplelib::SampleConfig".into(),
        has_serde: true,
        ..crate::core::ir::TypeDef::default()
    }
}

/// The release-blocking defect: a configured argument whose declared `type` contradicts the
/// target parameter's declared type was rendered by value shape alone.
///
/// `configure` takes one parameter the C ABI exports as `AlefHandle` (an unsigned integer),
/// and the fixture maps a JSON *object* onto it through an `args` entry left at the default
/// `type = "string"`. Because `args` is non-empty the arity is satisfied, so the empty-`args`
/// refusal never fires; nothing else ever compared the entry's type against the parameter's,
/// so `json_to_c` stringified the object and the emitter published
/// `sample_configure("{\"cache_dir\":...}")` -- `clang -fsyntax-only` rejects it with
/// `incompatible pointer to integer conversion`. A visible coverage gap beats a snippet that
/// cannot compile, so the emitter must refuse. ~keep
#[test]
fn string_literal_configured_against_a_handle_parameter_is_refused_not_emitted() {
    let fixture = Fixture {
        id: "configure_cache_dir".into(),
        description: "Configure the cache directory".into(),
        input: serde_json::json!({"config": {"cache_dir": "/tmp/sample_cache"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "configure".into();
    e2e.call.returns_void = true;
    e2e.call.args = vec![required_arg("config", "input.config", "string")];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            function: Some("sample_configure".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "configure".into(),
        params: vec![ParamDef {
            name: "config".into(),
            ty: TypeRef::Named("SampleConfig".into()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    }];
    let type_defs = [handle_param_config_type()];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let error = match render_c_snippet(&fixture, &e2e, &config, &type_defs, &functions) {
        Ok(rendered) => panic!(
            "a JSON object must never be lowered into a handle parameter, but the emitter published:\n{rendered}"
        ),
        Err(error) => format!("{error:#}"),
    };

    assert!(error.contains("sample_configure"), "must name the call: {error}");
    assert!(
        error.contains("`config`"),
        "must name the parameter it refused to fill: {error}"
    );
    assert!(
        error.contains("AlefHandle"),
        "must name the C type the parameter actually has: {error}"
    );
    assert!(
        error.contains("SampleConfig"),
        "must name the declared Rust type behind that handle: {error}"
    );
    assert!(
        error.contains("cache_dir"),
        "must quote the offending value so the operator can find the `args` entry: {error}"
    );
    assert!(
        error.contains("json_object"),
        "must name the configuration change that constructs the handle: {error}"
    );
}

/// The control that stops the fix from passing by refusing everything: the same call shape,
/// the same `type = "string"` entry, but a parameter the core really declares as a `String`.
/// The literal is the correct lowering there, so the snippet must still emit byte-for-byte
/// what it always emitted -- and must still compile. A refusal keyed on the value's JSON
/// shape rather than on the parameter's type would fail here. ~keep
#[test]
fn correctly_typed_string_argument_still_emits_against_a_string_parameter() {
    let fixture = Fixture {
        id: "configure_cache_dir".into(),
        description: "Configure the cache directory".into(),
        input: serde_json::json!({"config": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "configure".into();
    e2e.call.returns_void = true;
    e2e.call.args = vec![required_arg("config", "input.config", "string")];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            function: Some("sample_configure".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "configure".into(),
        params: vec![ParamDef {
            name: "config".into(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    }];
    let type_defs = [handle_param_config_type()];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let rendered =
        render_c_snippet(&fixture, &e2e, &config, &type_defs, &functions).expect("string-parameter snippet renders");

    assert!(
        rendered.contains("sample_configure(\"/tmp/sample_cache\");"),
        "a correctly typed string argument must still emit its literal:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "int32_t sample_configure(const char *config);\n",
        ),
    );
}

/// The second control, at the exact parameter type the test above refuses: the refusal is
/// keyed on the *lowering*, not on the parameter being a handle. Declaring the entry
/// `type = "json_object"` makes the free-function path construct the handle with `from_json`
/// first, so a real handle expression -- never a literal -- reaches the parameter, and the
/// emitted translation unit compiles against the exported header. ~keep
#[test]
fn json_object_argument_still_constructs_a_handle_for_the_same_parameter() {
    let fixture = Fixture {
        id: "process_with_config".into(),
        description: "Process with a configuration".into(),
        input: serde_json::json!({"config": {"cache_dir": "/tmp/sample_cache"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    e2e.call.args = vec![json_arg("config", "input.config", "SampleConfig")];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "process".into(),
        params: vec![ParamDef {
            name: "config".into(),
            ty: TypeRef::Named("SampleConfig".into()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("SampleReceipt".into()),
        ..FunctionDef::default()
    }];
    let type_defs = [
        handle_param_config_type(),
        crate::core::ir::TypeDef {
            name: "SampleReceipt".into(),
            rust_path: "samplelib::SampleReceipt".into(),
            ..crate::core::ir::TypeDef::default()
        },
    ];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let rendered =
        render_c_snippet(&fixture, &e2e, &config, &type_defs, &functions).expect("json_object snippet renders");

    assert!(
        rendered.contains("sample_sample_config_from_json"),
        "the handle must be constructed before the call:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_process(config_handle)"),
        "the constructed handle must be what reaches the parameter:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_process(\""),
        "no string literal may reach a handle-typed parameter:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_sample_config_from_json(const char *json);\n",
            "void sample_sample_config_free(SAMPLEAlefHandle value);\n",
            "SAMPLEAlefHandle sample_process(SAMPLEAlefHandle config);\n",
            "void sample_sample_receipt_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

/// The release-blocking defect's actual residual: the control test above proves the
/// free-function (non-`returns_void`) path constructs a handle correctly, but the
/// `returns_void` snippet path in `render_snippet_body` (`c/test_function.rs`) took an early
/// return before that construction ever ran, passing `build_args_string_c` an empty handle
/// map. A `json_object` arg onto a handle parameter therefore fell through to `json_to_c`'s
/// literal fallback -- the same `incompatible pointer to integer conversion` this whole family
/// of fixes exists to prevent, just reached through the void-call branch instead of the
/// legacy-call one. ~keep
#[test]
fn returns_void_call_constructs_a_handle_for_a_json_object_arg_onto_a_handle_parameter() {
    let fixture = Fixture {
        id: "configure_custom_dir".into(),
        description: "Configure the cache directory".into(),
        input: serde_json::json!({"config": {"cache_dir": "/tmp/sample_cache"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "configure".into();
    e2e.call.returns_void = true;
    e2e.call.args = vec![json_arg("config", "input.config", "SampleConfig")];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            function: Some("sample_configure".into()),
            ..Default::default()
        },
    );
    let functions = [FunctionDef {
        name: "configure".into(),
        params: vec![ParamDef {
            name: "config".into(),
            ty: TypeRef::Named("SampleConfig".into()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        ..FunctionDef::default()
    }];
    let type_defs = [handle_param_config_type()];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let rendered = render_c_snippet(&fixture, &e2e, &config, &type_defs, &functions)
        .expect("a json_object arg onto a handle parameter must render, not refuse");

    assert!(
        rendered.contains("sample_sample_config_from_json"),
        "the handle must be constructed before the call:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_configure(config_handle);"),
        "the constructed handle must be what reaches the parameter:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_configure(\""),
        "no string literal may reach a handle-typed parameter:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_sample_config_from_json(const char *json);\n",
            "void sample_sample_config_free(SAMPLEAlefHandle value);\n",
            "void sample_configure(SAMPLEAlefHandle config);\n",
        ),
    );
}

/// The other candidate site the release-blocking defect could have lived in: the
/// client-method byte-buffer pattern (`render_bytes_test_function` in `c/call_patterns.rs`),
/// which builds its argument list independently of `build_args_string_c`/`TargetParams`
/// entirely. Unlike the `returns_void` snippet path, this one already constructs the handle
/// unconditionally for every `json_object` arg -- this test pins that as a control so a future
/// change to this path cannot silently regress it back to a literal. ~keep
#[test]
fn bytes_result_client_method_still_constructs_a_handle_for_a_json_object_arg() {
    let fixture = Fixture {
        id: "transcribe_bytes".into(),
        description: "Transcribe audio".into(),
        input: serde_json::json!({"request": {"text": "hello"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "transcribe".into();
    e2e.call.result_is_bytes = true;
    e2e.call.options_type = Some("SampleTranscribeRequest".into());
    e2e.call.args = vec![json_arg("request", "input.request", "SampleTranscribeRequest")];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            client_factory: Some("client_new".into()),
            ..Default::default()
        },
    );
    let type_defs = [crate::core::ir::TypeDef {
        name: "SampleClient".into(),
        rust_path: "samplelib::SampleClient".into(),
        is_opaque: true,
        ..crate::core::ir::TypeDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let rendered =
        render_c_snippet(&fixture, &e2e, &config, &type_defs, &[]).expect("bytes-result client snippet renders");

    assert!(
        rendered.contains("sample_sample_transcribe_request_from_json"),
        "the request handle must be constructed before the call:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_default_client_transcribe(client, sample_transcribe_request_handle"),
        "the constructed handle must be what reaches the call:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_client_new(const char *api_key, const char *base_url, ",
            "uint64_t timeout_ms, uint32_t max_retries, const char *user_agent);\n",
            "void sample_default_client_free(SAMPLEAlefHandle client);\n",
            "SAMPLEAlefHandle sample_sample_transcribe_request_from_json(const char *json);\n",
            "void sample_sample_transcribe_request_free(SAMPLEAlefHandle value);\n",
            "int32_t sample_default_client_transcribe(SAMPLEAlefHandle client, SAMPLEAlefHandle request, ",
            "uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap);\n",
            "void sample_free_bytes(uint8_t *ptr, uintptr_t len, uintptr_t cap);\n",
        ),
    );
}
