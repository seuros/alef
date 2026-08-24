//! Regression coverage for the C batch-input calling convention.
//!
//! An `args` entry typed `json_object` says what the *fixture value* looks like — an object, or
//! an array of them. It says nothing about how the parameter crosses the C ABI, and the C ABI
//! splits the two cases: `backends::ffi::type_map` exports a bare `Named`/`Optional<Named>` as
//! the opaque `AlefHandle`, while `Vec<_>`, `Map<_, _>` and `Json` cross as the serialized JSON
//! text in a `*const c_char`. Constructing a handle for a batch parameter therefore contradicts
//! the header alef itself generated (`incompatible integer to pointer conversion passing
//! '{PREFIX}AlefHandle' to parameter of type 'const char *'`).
//!
//! Both arms are asserted here, and both are compiled against a neutral header that declares the
//! parameter the way the FFI backend actually would — a rendering assertion alone would not
//! notice the two generators disagreeing again.
//!
//! ~keep A submodule of `optional_arg` (the seam that owns the handle-versus-pointer decision)
//! rather than a new entry in `c.rs`: `c.rs`, `test_function.rs` and `snippet_regressions.rs` are
//! all over the repo's 1,000-line cap and may not grow — see `file-modularization` in CLAUDE.md
//! and the sibling `call_patterns/batch_url_regression_tests`.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use crate::e2e::codegen::c::render_c_snippet;
use crate::e2e::codegen::c::snippet_regressions::compile_snippet;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

/// A `json_object` arg with no `element_type`, so `resolve_call_info`'s IR backfill supplies it —
/// the path the defect travelled.
fn json_arg(name: &str, field: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn c_override() -> crate::core::config::e2e::CallOverride {
    crate::core::config::e2e::CallOverride {
        header: Some("sample_ffi.h".into()),
        ..Default::default()
    }
}

fn sample_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    }
}

/// REPRODUCTION: a batch parameter (`Vec<ItemInput>`) is declared `const char *` by the FFI
/// backend, so the snippet must pass the serialized JSON array — not a handle built from the
/// element type. Before the fix this rendered
/// `SAMPLEAlefHandle items_handle = sample_item_input_from_json("[...]");` followed by
/// `sample_process_batch(items_handle)`, which does not compile against the generated header.
#[test]
fn a_batch_input_declared_const_char_is_passed_as_the_json_string_not_a_handle() {
    let fixture = Fixture {
        id: "process_batch".into(),
        description: "Process a batch of items".into(),
        input: serde_json::json!({"items": [{"text": "a"}, {"text": "b"}]}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process_batch".into();
    e2e.call.args = vec![json_arg("items", "input.items")];
    e2e.call.overrides.insert("c".into(), c_override());
    let functions = [FunctionDef {
        name: "process_batch".into(),
        params: vec![ParamDef {
            name: "items".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named("ItemInput".into()))),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("BatchReport".into()),
        ..FunctionDef::default()
    }];
    let rendered = render_c_snippet(&fixture, &e2e, &sample_config(), &[], &functions).expect("batch snippet renders");

    assert!(
        rendered.contains(r#"sample_process_batch("[{\"text\":\"a\"},{\"text\":\"b\"}]")"#),
        "the batch input must reach the call as the serialized JSON string:\n{rendered}"
    );
    assert!(
        !rendered.contains("items_handle"),
        "a `const char *` batch parameter must not be wrapped in a handle:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_item_input_from_json"),
        "the element type's `from_json` constructor must not be called for a batch parameter:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_process_batch(const char *items);\n",
            "void sample_batch_report_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

/// CONTROL: the arm that must not regress. A genuine JSON-object argument — one whose parameter
/// the Rust core declares as a bare `Named` type, which the FFI backend exports as `AlefHandle` —
/// still gets its handle constructed, asserted and freed. The fix distinguishes the two cases by
/// the declared parameter; it does not stop building handles.
#[test]
fn a_json_object_arg_declared_a_named_type_still_constructs_a_handle() {
    let fixture = Fixture {
        id: "convert_source".into(),
        description: "Convert a source".into(),
        input: serde_json::json!({"source": {"text": "a"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "convert".into();
    e2e.call.args = vec![json_arg("source", "input.source")];
    e2e.call.overrides.insert("c".into(), c_override());
    let functions = [FunctionDef {
        name: "convert".into(),
        params: vec![ParamDef {
            name: "source".into(),
            ty: TypeRef::Named("SourceInput".into()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("ConvertReport".into()),
        ..FunctionDef::default()
    }];
    let rendered =
        render_c_snippet(&fixture, &e2e, &sample_config(), &[], &functions).expect("named-arg snippet renders");

    assert!(
        rendered.contains(r#"SAMPLEAlefHandle source_handle = sample_source_input_from_json("{\"text\":\"a\"}")"#),
        "a handle-typed parameter must still have its handle constructed:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_convert(source_handle)"),
        "the constructed handle must be what reaches the call:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_source_input_free(source_handle)"),
        "the constructed handle must still be freed:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_source_input_from_json(const char *json);\n",
            "void sample_source_input_free(SAMPLEAlefHandle value);\n",
            "SAMPLEAlefHandle sample_convert(SAMPLEAlefHandle source);\n",
            "void sample_convert_report_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

/// The rule is the declared parameter's C shape, not "is it an array": a `serde_json::Value`
/// parameter also crosses as `*const c_char`. This arm also covers a case that used to abort
/// generation outright — `named_type` answers `None` for `TypeRef::Json`, leaving `element_type`
/// unset, which tripped `build_json_object_arg_handles`' "no resolvable type" panic whenever no
/// `options_type` fallback was configured.
#[test]
fn a_free_form_json_parameter_is_passed_as_the_json_string() {
    let fixture = Fixture {
        id: "annotate".into(),
        description: "Annotate a payload".into(),
        input: serde_json::json!({"payload": {"text": "a"}}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "annotate".into();
    e2e.call.args = vec![json_arg("payload", "input.payload")];
    e2e.call.overrides.insert("c".into(), c_override());
    let functions = [FunctionDef {
        name: "annotate".into(),
        params: vec![ParamDef {
            name: "payload".into(),
            ty: TypeRef::Json,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("AnnotationReport".into()),
        ..FunctionDef::default()
    }];
    let rendered =
        render_c_snippet(&fixture, &e2e, &sample_config(), &[], &functions).expect("json-param snippet renders");

    assert!(
        rendered.contains(r#"sample_annotate("{\"text\":\"a\"}")"#),
        "a `serde_json::Value` parameter must receive the serialized JSON string:\n{rendered}"
    );
    assert!(
        !rendered.contains("payload_handle"),
        "a `const char *` JSON parameter must not be wrapped in a handle:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_annotate(const char *payload);\n",
            "void sample_annotation_report_free(SAMPLEAlefHandle result);\n",
        ),
    );
}
