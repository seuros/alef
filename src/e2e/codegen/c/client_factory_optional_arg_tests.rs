//! Regression coverage for the `client_factory` ("default_client") call path's optional-arg
//! sentinel selection -- the shape reported against liter-llm's `list_files(client, NULL)`:
//! a handle-typed optional parameter with no configured `arg_type` (which defaults to
//! `"string"`) rendered `NULL` instead of `0`, which does not compile
//! (`incompatible pointer to integer conversion`) against the `AlefHandle`
//! (`unsigned long long`) parameter the FFI header declares.
//!
//! `render_c_snippet` (the doc-snippet path) and `render_test_file` (the real e2e-test-file
//! emitter, which also drives `test_apps/`) both eventually call
//! `test_function::render_test_function_impl`'s `client_factory` branch and both must agree,
//! since a fix that only reached one of them was the actual shape of the historic defect (see
//! `c::optional_arg`'s module doc). This module pins both against the same fixture and IR.

use super::snippet_regressions::compile_snippet;
use super::*;
use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};

/// A `Client` opaque type declaring one method, `list_files(cursor: Option<Cursor>, note:
/// Option<String>) -> FileList` -- mirroring the real regression's `list_files(client,
/// cursor)` shape. Neither `cursor` nor `note` gets an explicit `arg_type` in the fixture
/// config below, so both default to `"string"` (`default_arg_type`) -- the exact
/// under-specified authoring state that produced the historic bug for the handle-typed one.
fn client_type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Client".into(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "list_files".into(),
            params: vec![
                ParamDef {
                    name: "cursor".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("Cursor".into()))),
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "note".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("FileList".into()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    }]
}

/// `cursor`/`note` args with NO explicit `type =` -- `default_arg_type` fills in `"string"`
/// for both, same as a fixture author who never configured `arg_type` at all.
fn list_files_args() -> Vec<crate::e2e::config::ArgMapping> {
    vec![
        crate::e2e::config::ArgMapping {
            name: "cursor".into(),
            field: "input.cursor".into(),
            arg_type: "string".into(),
            optional: true,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        },
        crate::e2e::config::ArgMapping {
            name: "note".into(),
            field: "input.note".into(),
            arg_type: "string".into(),
            optional: true,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        },
    ]
}

fn list_files_e2e_config() -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "list_files".into();
    e2e.call.args = list_files_args();
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            client_factory: Some("create_client".into()),
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    e2e
}

fn list_files_fixture() -> Fixture {
    Fixture {
        id: "list_files_defaults".into(),
        description: "List files with no cursor or note".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    }
}

const HEADER: &str = concat!(
    "#include <stdint.h>\n",
    "typedef uint64_t SAMPLEAlefHandle;\n",
    "SAMPLEAlefHandle sample_create_client(const char *api_key, const char *base_url, ",
    "uint64_t timeout_ms, uint32_t max_retries, const char *user_agent);\n",
    "void sample_default_client_free(SAMPLEAlefHandle client);\n",
    "SAMPLEAlefHandle sample_default_client_list_files(SAMPLEAlefHandle client, ",
    "SAMPLEAlefHandle cursor, const char *note);\n",
    "void sample_file_list_free(SAMPLEAlefHandle result);\n",
);

/// The doc-snippet surface: `render_c_snippet` -> `render_snippet_body` ->
/// `render_test_function_impl`'s `client_factory` branch.
#[test]
fn snippet_path_types_the_omitted_client_method_args() {
    let fixture = list_files_fixture();
    let e2e = list_files_e2e_config();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let type_defs = client_type_defs();

    let rendered = render_c_snippet(&fixture, &e2e, &config, &type_defs, &[]).expect("snippet renders");

    assert!(
        rendered.contains("sample_default_client_list_files(client, 0, NULL)"),
        "expected the handle-typed `cursor` to render `0` and the string-typed `note` to \
         render `NULL`, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_default_client_list_files(client, NULL, NULL)"),
        "the handle-typed `cursor` must never render the pointer sentinel `NULL`:\n{rendered}"
    );
    compile_snippet(&rendered, "sample_ffi.h", HEADER);
}

/// The real e2e-test-file emitter surface (which also drives `test_apps/`):
/// `render_test_file` -> `render_test_function_impl`'s `client_factory` branch, through the
/// same `TargetParams` resolution `render_snippet_body` performs -- the two must agree.
#[test]
fn e2e_test_file_path_types_the_omitted_client_method_args() {
    let fixture = list_files_fixture();
    let e2e = list_files_e2e_config();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let type_defs = client_type_defs();
    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let ir = CallIr {
        functions: &[],
        type_defs: &type_defs,
    };

    let rendered = render_test_file(
        "files",
        &[&fixture],
        "sample_ffi.h",
        "sample",
        "result",
        &e2e,
        "c",
        &field_resolver,
        &config,
        &type_defs,
        &[],
        &[],
        ir,
    )
    .expect("test file renders");

    assert!(
        rendered.contains("sample_default_client_list_files(client, 0, NULL)"),
        "expected the handle-typed `cursor` to render `0` and the string-typed `note` to \
         render `NULL`, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_default_client_list_files(client, NULL, NULL)"),
        "the handle-typed `cursor` must never render the pointer sentinel `NULL`:\n{rendered}"
    );
}
