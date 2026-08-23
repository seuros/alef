//! Docs-snippet rendering for typed DTO inputs: nested files, generics, and wire names.

use super::*;
use crate::e2e::config::{CallConfig, CallOverride};

#[test]
fn snippet_reads_nested_typed_dto_files() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "document_input",
        "description": "Read a document",
        "input": {"request": {"content": "ignored"}},
        "assertions": [],
        "docs": {
            "topic": "documents",
            "presentation": {"files": [{"field": "/request/content", "path": "document.pdf"}]}
        }
    }))
    .expect("fixture");
    let mut call = CallConfig {
        function: "process".into(),
        args: vec![crate::e2e::config::ArgMapping {
            name: "request".into(),
            field: "request".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin".into(),
        CallOverride {
            options_type: Some("DocumentRequest".into()),
            ..CallOverride::default()
        },
    );

    let body = render_snippet_body(
        &fixture.docs_call_fixture(),
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        false,
    )
    .expect("snippet renders");

    assert!(
        body.contains("Files.readAllBytes(java.nio.file.Path.of(\"document.pdf\"))"),
        "{body}"
    );
    assert!(body.contains("Base64.getEncoder().encodeToString"), "{body}");
    assert!(body.contains("DocumentRequest::class.java"), "{body}");
}

#[test]
fn snippet_deserializes_generic_typed_dto_without_file_metadata() {
    let fixture = Fixture {
        id: "document_input".into(),
        description: "Process a document".into(),
        input: serde_json::json!({"kind": "uri", "uri": "document.txt"}),
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "process".into(),
        args: vec![crate::e2e::config::ArgMapping {
            name: "input".into(),
            field: "input".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: Some("DocumentInput".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin_android".into(),
        CallOverride {
            options_type: Some("ExtractionConfig".into()),
            ..CallOverride::default()
        },
    );

    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &ResolvedCrateConfig::default(),
        &[],
        &[],
        true,
    )
    .expect("snippet renders");

    assert!(body.contains("val input = mapper.readValue("), "{body}");
    assert!(body.contains("DocumentInput::class.java"), "{body}");
    assert!(!body.contains("ExtractionConfig::class.java"), "{body}");
    assert!(body.contains(".process(input)"), "{body}");
    assert!(body.contains("jacksonObjectMapper"), "{body}");
}

#[test]
fn snippet_uses_nested_centralized_wire_names() {
    let fixture = Fixture {
        id: "document_input".into(),
        description: "Process a document".into(),
        input: serde_json::json!({"request_id": "one", "details": {"page_count": 2}}),
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "process".into(),
        args: vec![crate::e2e::config::ArgMapping {
            name: "input".into(),
            field: "input".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }],
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin_android".into(),
        CallOverride {
            options_type: Some("DocumentInput".into()),
            ..CallOverride::default()
        },
    );
    let type_defs = vec![
        crate::core::ir::TypeDef {
            name: "DocumentInput".into(),
            fields: vec![
                crate::core::ir::FieldDef {
                    name: "request_id".into(),
                    serde_rename: Some("request-id".into()),
                    ..Default::default()
                },
                crate::core::ir::FieldDef {
                    name: "details".into(),
                    ty: crate::core::ir::TypeRef::Named("DocumentDetails".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        crate::core::ir::TypeDef {
            name: "DocumentDetails".into(),
            serde_rename_all: Some("camelCase".into()),
            fields: vec![crate::core::ir::FieldDef {
                name: "page_count".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    ];

    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &ResolvedCrateConfig::default(),
        &type_defs,
        &[],
        true,
    )
    .expect("snippet renders");

    assert!(body.contains(r#"\"request-id\":\"one\""#), "{body}");
    assert!(body.contains(r#"\"pageCount\":2"#), "{body}");
    assert!(body.contains("val mapper = jacksonObjectMapper()"), "{body}");
}
