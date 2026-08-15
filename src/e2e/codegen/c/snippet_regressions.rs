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
