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
