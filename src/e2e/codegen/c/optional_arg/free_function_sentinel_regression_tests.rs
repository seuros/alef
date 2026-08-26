//! Regression coverage for the free-function/typed-arg C argument path
//! (`assertions.rs::build_args_string_c`) picking the right "none" sentinel for an omitted
//! optional argument.
//!
//! `resolve_optional_sentinel` already answers this correctly and has unit coverage of its own
//! in the parent module, but a correct answer nobody calls does not fix anything: before this
//! fix, `build_args_string_c`'s explicit-null arm called `c_optional_sentinel(&arg.arg_type)`
//! directly, skipping the IR-declared-type check entirely. An optional handle-typed parameter
//! with no configured `arg_type` (the common case -- `arg_type` defaults to `"string"`) rendered
//! `NULL`, which does not compile against the `AlefHandle` (`uint64_t`) parameter the FFI header
//! actually declares (`-Wint-conversion`). Exercising the end-to-end render, not just the seam
//! function, is what proves the call site is actually wired to it.
//!
//! ~keep A submodule of `optional_arg` rather than a new entry in `c.rs` or `assertions.rs`:
//! both are already over the repo's 1,000-line cap and may not grow -- see `file-modularization`
//! in CLAUDE.md and the sibling `batch_input_regression_tests`.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use crate::e2e::codegen::c::render_c_snippet;
use crate::e2e::codegen::c::snippet_regressions::compile_snippet;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

/// An arg with no `arg_type` configured, matching `default_arg_type`'s `"string"` default -- the
/// state a fixture author leaves an entry in when they never set `arg_type` explicitly, which is
/// the path the defect travelled.
fn unconfigured_optional_arg(name: &str, field: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "string".into(),
        optional: true,
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

/// REPRODUCTION: an omitted optional handle-typed parameter must render the `0` sentinel, not
/// `NULL`, even though `arg_type` was never configured for it.
#[test]
fn an_omitted_optional_handle_param_with_unconfigured_arg_type_renders_zero() {
    let fixture = Fixture {
        id: "list_batches".into(),
        description: "List batches, optionally after a cursor".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "list_batches".into();
    e2e.call.args = vec![unconfigured_optional_arg("cursor", "input.cursor")];
    e2e.call.overrides.insert("c".into(), c_override());
    let functions = [FunctionDef {
        name: "list_batches".into(),
        params: vec![ParamDef {
            name: "cursor".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("Cursor".into()))),
            optional: true,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("BatchReport".into()),
        ..FunctionDef::default()
    }];
    let rendered =
        render_c_snippet(&fixture, &e2e, &sample_config(), &[], &functions).expect("list_batches snippet renders");

    assert!(
        rendered.contains("sample_list_batches(0)"),
        "an omitted optional handle parameter must pass the `0` handle sentinel:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_list_batches(NULL)"),
        "`NULL` does not compile against the `AlefHandle` (uint64_t) parameter the header declares:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_list_batches(SAMPLEAlefHandle cursor);\n",
            "void sample_batch_report_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

/// CONTROL: the arm that must not regress. An omitted optional parameter that genuinely crosses
/// the C ABI as a pointer (a `const char *`, never a scalar `AlefHandle`) must keep the `NULL`
/// sentinel.
#[test]
fn an_omitted_optional_string_param_still_renders_null() {
    let fixture = Fixture {
        id: "list_named".into(),
        description: "List items, optionally filtered by a name prefix".into(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "list_named".into();
    e2e.call.args = vec![unconfigured_optional_arg("prefix", "input.prefix")];
    e2e.call.overrides.insert("c".into(), c_override());
    let functions = [FunctionDef {
        name: "list_named".into(),
        params: vec![ParamDef {
            name: "prefix".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::String)),
            optional: true,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("NamedReport".into()),
        ..FunctionDef::default()
    }];
    let rendered =
        render_c_snippet(&fixture, &e2e, &sample_config(), &[], &functions).expect("list_named snippet renders");

    assert!(
        rendered.contains("sample_list_named(NULL)"),
        "an omitted optional string parameter must keep the `NULL` pointer sentinel:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_list_named(0)"),
        "a genuine pointer parameter must not regress to the handle sentinel:\n{rendered}"
    );
    compile_snippet(
        &rendered,
        "sample_ffi.h",
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_list_named(const char *prefix);\n",
            "void sample_named_report_free(SAMPLEAlefHandle result);\n",
        ),
    );
}
