use super::*;

fn assert_strict_typescript_compiles(source: &str) {
    let directory = tempfile::tempdir().expect("temporary TypeScript project");
    let source_path = directory.path().join("snippet.ts");
    std::fs::write(&source_path, source).expect("write TypeScript regression source");
    let Ok(output) = std::process::Command::new("tsc")
        .args([
            "--strict",
            "--noUncheckedIndexedAccess",
            "--noEmit",
            "--target",
            "ES2022",
        ])
        .arg(&source_path)
        .output()
    else {
        return;
    };
    assert!(
        output.status.success(),
        "strict TypeScript rejected generated snippet:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_strict_typescript_rejects(source: &str) {
    let directory = tempfile::tempdir().expect("temporary TypeScript project");
    let source_path = directory.path().join("snippet.ts");
    std::fs::write(&source_path, source).expect("write TypeScript regression source");
    let Ok(output) = std::process::Command::new("tsc")
        .args([
            "--strict",
            "--noUncheckedIndexedAccess",
            "--noEmit",
            "--target",
            "ES2022",
        ])
        .arg(&source_path)
        .output()
    else {
        return;
    };
    assert!(
        !output.status.success(),
        "expected strict TypeScript to reject the flattened literal, but it compiled:\n{source}"
    );
}

/// `Message` is a `#[serde(tag = "role")]` enum with one tuple variant, `User(UserMessage)`
/// — the shape from the E3 snippet/binding-agreement defect (108 x TS2353 failures against
/// a real crate's `Message` union).
fn message_enum_def() -> EnumDef {
    EnumDef {
        name: "Message".into(),
        serde_tag: Some("role".into()),
        serde_rename_all: Some("snake_case".into()),
        variants: vec![crate::core::ir::EnumVariant {
            name: "User".into(),
            is_tuple: true,
            fields: vec![crate::core::ir::FieldDef {
                name: "_0".into(),
                ty: TypeRef::Named("UserMessage".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn user_message_type_def() -> TypeDef {
    TypeDef {
        name: "UserMessage".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "content".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn node_tagged_enum_variant_nests_payload_under_synthesized_field() {
    let enums = [message_enum_def()];
    let type_defs = [user_message_type_def()];
    let expression = ts_builder_expression(
        serde_json::json!({"role": "user", "content": "Hello"})
            .as_object()
            .expect("object"),
        "Message",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &enums,
        "",
        &[],
        &mut Default::default(),
    );

    assert_eq!(
        expression,
        "{ role: \"user\", user: { content: \"Hello\" } } as Message"
    );
}

/// The real cross-generator guard for the E3 message-shape defect: builds the snippet
/// object literal with the same `ts_builder_expression` the e2e generator uses, generates
/// the `.d.ts` union type with the exact production function napi's backend calls
/// (`internal_tagged_union_dts_lines`, re-exported from `backends::napi`), and typechecks
/// the snippet against that generated union with `tsc --strict`.
///
/// This proves the snippet and the binding agree on the *nesting* shape (payload under a
/// synthesized per-variant field, not flattened alongside the tag). It does NOT cover: (a)
/// whether the real napi Rust struct actually accepts this literal at runtime (that needs a
/// compiled `.node` binary, out of scope for a unit test), (b) multi-field tuple variants,
/// struct variants, or adjacently-tagged (`serde_content`) enums, which take different
/// nesting rules not exercised here, and (c) any language/backend other than node.
#[test]
fn node_tagged_enum_snippet_typechecks_against_the_generated_dts_union() {
    let enums = [message_enum_def()];
    let type_defs = [user_message_type_def()];
    let expression = ts_builder_expression(
        serde_json::json!({"role": "user", "content": "Hello"})
            .as_object()
            .expect("object"),
        "Message",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &enums,
        "",
        &[],
        &mut Default::default(),
    );

    let dts = crate::backends::napi::internal_tagged_union_dts_lines(&enums[0], "Message").join("\n");
    let source = format!(
        "interface UserMessage {{ content: string }}\n{dts}\nconst message: Message = {expression};\nvoid message;\n"
    );
    assert_strict_typescript_compiles(&source);
}

/// Negative control proving the guard above is not vacuous: the pre-fix flattened shape
/// (`{ role: 'user', content: 'Hello' }`, the actual output before this change) is rejected
/// by `tsc` against the same generated `.d.ts` union that the positive test compiles clean
/// against.
#[test]
fn node_flattened_message_literal_fails_the_generated_dts_union() {
    let enums = [message_enum_def()];
    let dts = crate::backends::napi::internal_tagged_union_dts_lines(&enums[0], "Message").join("\n");
    let flattened = "{ role: \"user\", content: \"Hello\" } as Message";
    let source = format!(
        "interface UserMessage {{ content: string }}\n{dts}\nconst message: Message = {flattened};\nvoid message;\n"
    );
    assert_strict_typescript_rejects(&source);
}

#[test]
fn node_typed_objects_use_importable_enum_members() {
    let expression = ts_builder_expression(
        serde_json::json!({"kind": "uri"}).as_object().expect("object"),
        "DocumentInput",
        &Default::default(),
        "node",
        &[("kind".into(), "InputKind".into())].into_iter().collect(),
        &Default::default(),
        &[],
        &[],
        "",
        &[],
        &mut Default::default(),
    );

    assert_eq!(expression, "{ kind: InputKind.Uri } as DocumentInput");
}

#[test]
fn node_typed_objects_lower_bytes_and_enums_from_ir() {
    let type_defs = [TypeDef {
        name: "DocumentInput".into(),
        fields: vec![
            crate::core::ir::FieldDef {
                name: "bytes".into(),
                ty: crate::core::ir::TypeRef::Bytes,
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: crate::core::ir::TypeRef::Named("InputKind".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let enums = [EnumDef {
        name: "InputKind".into(),
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"bytes": [72, 105], "kind": "bytes"})
            .as_object()
            .expect("object"),
        "DocumentInput",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &enums,
        "",
        &[],
        &mut Default::default(),
    );

    assert_eq!(
        expression,
        "{ bytes: Uint8Array.from([72, 105]), kind: InputKind.Bytes } as DocumentInput"
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("content".to_string(), "results[0].content".to_string());
    let optional = ["results".to_string()].into_iter().collect();
    let resolver = crate::e2e::field_access::FieldResolver::new(
        &fields,
        &optional,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    let accessor = resolver.accessor("content", "node", "result");
    let source = format!(
        "enum InputKind {{ Bytes }}\ninterface DocumentInput {{ bytes: Uint8Array; kind: InputKind }}\ninterface Output {{ results?: Array<{{ content: string }}> }}\nconst input: DocumentInput = {expression};\ndeclare const result: Output;\nconst content: string | undefined = {accessor};\nvoid input; void content;\n"
    );
    assert_strict_typescript_compiles(&source);
}

#[test]
fn wasm_typed_objects_lower_bytes_and_enums_from_ir() {
    let type_defs = [TypeDef {
        name: "ExtractInput".into(),
        fields: vec![
            crate::core::ir::FieldDef {
                name: "bytes".into(),
                ty: crate::core::ir::TypeRef::Bytes,
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: crate::core::ir::TypeRef::Named("ExtractInputKind".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let enums = [EnumDef {
        name: "ExtractInputKind".into(),
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"bytes": [72, 105], "kind": "bytes"})
            .as_object()
            .expect("object"),
        "WasmExtractInput",
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &enums,
        "Wasm",
        &[],
        &mut Default::default(),
    );
    assert!(
        expression.contains("_u0.bytes = Uint8Array.from([72, 105])"),
        "{expression}"
    );
    assert!(
        expression.contains("_u0.kind = WasmExtractInputKind.Bytes"),
        "{expression}"
    );
    let source = format!(
        "enum WasmExtractInputKind {{ Bytes }}\nclass WasmExtractInput {{ static default(): WasmExtractInput {{ return new WasmExtractInput(); }} bytes!: Uint8Array; kind!: WasmExtractInputKind; }}\nconst input: WasmExtractInput = {expression};\nvoid input;\n"
    );
    assert_strict_typescript_compiles(&source);
}

/// Regression for the #468 byte-payload typing defect: a `bytes`-typed field whose fixture
/// value is a JSON *string* (a file path, here) used to reach the WASM setter builder's
/// generic fallback and render as the bare fixture string (`_u0.bytes = "pdf/fake_memo.pdf";`)
/// — not assignable to the generated binding's `bytes: Uint8Array` setter. The builder must
/// ask `ts_bytes_value_expression` instead of assuming the array-shaped value the sibling test
/// `wasm_typed_objects_lower_bytes_and_enums_from_ir` exercises.
#[test]
fn wasm_typed_objects_lower_bytes_file_path_from_ir() {
    let type_defs = [TypeDef {
        name: "ExtractInput".into(),
        fields: vec![
            crate::core::ir::FieldDef {
                name: "bytes".into(),
                ty: crate::core::ir::TypeRef::Bytes,
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "kind".into(),
                ty: crate::core::ir::TypeRef::Named("ExtractInputKind".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];
    let enums = [EnumDef {
        name: "ExtractInputKind".into(),
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"bytes": "pdf/fake_memo.pdf", "kind": "bytes"})
            .as_object()
            .expect("object"),
        "WasmExtractInput",
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &enums,
        "Wasm",
        &[],
        &mut Default::default(),
    );
    assert!(
        expression.contains("_u0.bytes = await (await import(\"node:fs/promises\")).readFile(\"pdf/fake_memo.pdf\");"),
        "{expression}"
    );
    // Reading the fixture file is async — the IIFE wrapping the setters must be declared
    // `async` too, or the `await` inside it is a syntax error.
    assert!(expression.starts_with("await (async () =>"), "{expression}");
}

/// Negative control: a plain `String` field (not `TypeRef::Bytes`) whose value happens to look
/// like a file path must stay a quoted string literal. The gate is the field's declared IR
/// type, not a heuristic over the value's shape.
#[test]
fn wasm_typed_objects_leave_plain_string_field_quoted() {
    let type_defs = [TypeDef {
        name: "ExtractInput".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "filename".into(),
            ty: crate::core::ir::TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"filename": "pdf/fake_memo.pdf"})
            .as_object()
            .expect("object"),
        "WasmExtractInput",
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        "Wasm",
        &[],
        &mut Default::default(),
    );
    assert!(
        expression.contains("_u0.filename = \"pdf/fake_memo.pdf\";"),
        "a non-bytes String field must stay a quoted literal, not a file read: {expression}"
    );
    assert!(!expression.contains("readFile"), "{expression}");
}

/// Negative control: a genuinely `Vec<u32>`-typed field (not `TypeRef::Bytes`) with a numeric
/// array value must stay a plain JS array literal, not get wrapped as `Uint8Array.from(...)`.
#[test]
fn wasm_typed_objects_leave_number_array_field_as_array_literal() {
    let type_defs = [TypeDef {
        name: "Sample".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "weights".into(),
            ty: crate::core::ir::TypeRef::Vec(Box::new(crate::core::ir::TypeRef::Primitive(
                crate::core::ir::PrimitiveType::U32,
            ))),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"weights": [1, 2, 3]}).as_object().expect("object"),
        "WasmSample",
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        "Wasm",
        &[],
        &mut Default::default(),
    );
    assert!(
        expression.contains("_u0.weights = [1, 2, 3];"),
        "a non-bytes number[] field must stay a plain array literal, not Uint8Array.from: {expression}"
    );
    assert!(!expression.contains("Uint8Array"), "{expression}");
}

#[test]
fn wasm_untagged_data_enum_field_emits_raw_value_not_enum_member() {
    // `EmbeddingInput` is an untagged data enum: on the wire it is the bare
    // payload of whichever variant matched (here, a plain string), so the
    // fixture value "" is real input data, not the name of a `WasmEmbeddingInput`
    // member.
    let type_defs = [TypeDef {
        name: "EmbeddingRequest".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "input".into(),
            ty: crate::core::ir::TypeRef::Named("EmbeddingInput".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let enums = [EnumDef {
        name: "EmbeddingInput".into(),
        serde_untagged: true,
        variants: vec![crate::core::ir::EnumVariant {
            name: "Text".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "0".into(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            }],
            is_tuple: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"input": ""}).as_object().expect("object"),
        "WasmEmbeddingRequest",
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &enums,
        "Wasm",
        &[],
        &mut Default::default(),
    );
    assert!(expression.contains("_u0.input = \"\";"), "{expression}");
    assert!(!expression.contains("WasmEmbeddingInput."), "{expression}");
}

/// The filter site directly: a fixture key `SampleOptions` doesn't declare must refuse
/// generation rather than silently reach the emitted literal. See the `~keep` comment at the
/// filter in `mod.rs` for why this is a refusal and not a drop, and
/// `json_object_field_agreement_tests.rs` for the snippet/e2e cross-generator coverage this
/// unit test underpins.
#[test]
fn undeclared_key_panics_instead_of_reaching_the_literal() {
    let type_defs = [TypeDef {
        name: "SampleOptions".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "content".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ts_builder_expression(
            serde_json::json!({"content": "hello", "bogus": "oops"})
                .as_object()
                .expect("object"),
            "SampleOptions",
            &Default::default(),
            "node",
            &Default::default(),
            &Default::default(),
            &type_defs,
            &[],
            "",
            &[],
            &mut Default::default(),
        )
    }));
    let error = result.expect_err("an undeclared key must panic generation, not render silently");
    let message = error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(
        message.contains("bogus"),
        "panic message must name the offending key: {message}"
    );
    assert!(
        message.contains("SampleOptions"),
        "panic message must name the type: {message}"
    );
}

/// Negative control: a fully declared object must render exactly as before — the filter must
/// not reject or alter a fixture that only uses real fields.
#[test]
fn fully_declared_object_is_unaffected_by_the_filter() {
    let type_defs = [TypeDef {
        name: "SampleOptions".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "content".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"content": "hello"}).as_object().expect("object"),
        "SampleOptions",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        "",
        &[],
        &mut Default::default(),
    );
    assert_eq!(expression, "{ content: \"hello\" } as SampleOptions");
}

/// A `#[serde(flatten)]` field makes the owning struct's accepted key set open-ended — the
/// filter must not refuse a key that only the flattened target (not `SampleOptions` itself)
/// would recognise.
#[test]
fn flattened_struct_is_exempt_from_the_filter() {
    let type_defs = [TypeDef {
        name: "SampleOptions".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "extra".into(),
            ty: TypeRef::String,
            serde_flatten: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let expression = ts_builder_expression(
        serde_json::json!({"totally_unknown_key": "hello"})
            .as_object()
            .expect("object"),
        "SampleOptions",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        "",
        &[],
        &mut Default::default(),
    );
    assert_eq!(expression, "{ totallyUnknownKey: \"hello\" } as SampleOptions");
}

#[test]
fn node_and_wasm_typed_objects_read_documented_files() {
    let object = serde_json::json!({"bytes": "document.pdf"});
    let object = object.as_object().expect("object");
    let files = [crate::e2e::fixture::FixtureDocsFileInput {
        field: "/bytes".into(),
        path: "document.pdf".into(),
    }];
    for language in ["node", "wasm"] {
        let expression = ts_builder_expression(
            object,
            "DocumentInput",
            &Default::default(),
            language,
            &Default::default(),
            &Default::default(),
            &[],
            &[],
            "",
            &files,
            &mut Default::default(),
        );
        assert!(
            expression.contains("readFile(\"document.pdf\")"),
            "{language}: {expression}"
        );
        assert!(
            !expression.contains("bytes: \"document.pdf\""),
            "{language}: {expression}"
        );
        if language == "wasm" {
            assert!(expression.starts_with("await (async () =>"), "{expression}");
        }
    }
}

/// A fixture may key a field by its wire name rather than its Rust name, and that is correct
/// authoring, not a typo.
///
/// ~keep The declared-field guard originally compared against `field.name` alone, which would
/// have aborted generation for every `#[serde(rename)]`d field in every consumer's fixtures — a
/// guard that fires on correct input is worse than the TS2353 asymmetry it exists to close.
#[test]
fn a_field_keyed_by_its_wire_name_is_accepted() {
    let type_defs = [TypeDef {
        name: "Options".into(),
        fields: vec![crate::core::ir::FieldDef {
            name: "max_chars".into(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
            serde_rename: Some("maxCharacters".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    let expression = ts_builder_expression(
        serde_json::json!({"maxCharacters": 10}).as_object().expect("object"),
        "Options",
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        &type_defs,
        &[],
        "",
        &[],
        &mut Default::default(),
    );

    assert!(
        expression.contains("10"),
        "a key spelled with the field's wire name must build, not abort: {expression}"
    );
}
