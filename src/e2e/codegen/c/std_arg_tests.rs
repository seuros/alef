//! The FFI crate exports `<prefix>_<type>_from_json` / `_free` only for types the crate itself
//! defines. A `Vec<String>` argument resolves `element_type` to the std type `String`, which has
//! no such constructor -- the C ABI takes it as a plain `const char *` JSON string instead.

use super::snippet_regressions::compile_snippet;
use super::*;

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

fn render_prefetch_snippet(element_type: &str) -> String {
    let fixture = Fixture {
        id: "prefetch_empty_list".into(),
        description: "prefetch([]) is a no-op that succeeds".into(),
        input: serde_json::json!({ "languages": [] }),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "prefetch".into();
    e2e.call.args = vec![json_arg("languages", "input.languages", element_type)];
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("snippet renders")
}

/// A `Vec<String>` argument used to be materialised through
/// `sample_string_from_json(...)`/`sample_string_free(...)`, neither of which the FFI declares, so
/// the snippet failed to compile with "call to undeclared function". The JSON is passed straight
/// through as the `const char *` the ABI actually takes.
#[test]
fn a_std_typed_json_argument_is_passed_as_a_literal_not_a_handle() {
    let rendered = render_prefetch_snippet("String");

    assert!(
        !rendered.contains("sample_string_from_json"),
        "no `_from_json` constructor exists for a std type; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_string_free"),
        "no `_free` exists for a std type either; got:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"sample_prefetch("[]")"#),
        "the JSON should be spliced directly as the const char * argument; got:\n{rendered}"
    );

    compile_snippet(
        &rendered,
        "sample_ffi.h",
        // The header declares the handle-returning shape this fixture's return config implies;
        // the point under test is the *argument*, which must arrive as a `const char *` literal.
        concat!(
            "#include <stdint.h>\n",
            "typedef uint64_t SAMPLEAlefHandle;\n",
            "SAMPLEAlefHandle sample_prefetch(const char *languages);\n",
            "void sample_prefetch_free(SAMPLEAlefHandle result);\n",
        ),
    );
}

/// The control: a genuine crate-defined type still gets its typed handle, so the fix narrows the
/// std case only and does not disable the `element_type` backfill it sits next to.
#[test]
fn a_crate_defined_json_argument_still_builds_a_typed_handle() {
    let rendered = render_prefetch_snippet("LanguageSpec");

    assert!(
        rendered.contains("sample_language_spec_from_json"),
        "a crate-defined type must still construct a typed handle; got:\n{rendered}"
    );
    assert!(
        rendered.contains("sample_language_spec_free"),
        "and must still free it; got:\n{rendered}"
    );
}
