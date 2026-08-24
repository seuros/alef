//! The C ABI symbol a trait-bridge registry fixture names must come from the FFI
//! backend's own derivation, not from the language-agnostic `[e2e.calls.*] function`.
//!
//! `src/backends/ffi/trait_bridge/registration.rs` exports `{prefix}_clear_{trait_snake}`
//! and `{prefix}_unregister_{trait_snake}`, discarding the `clear_fn`/`unregister_fn`
//! config text's spelling. A crate that names `clear_fn = "clear_sample_backends"`
//! (plural) on a trait `SampleBackend` therefore ships `sample_clear_sample_backend`
//! (singular). Every fixture in this file sets the base `function` -- the shape a
//! well-formed `[e2e.calls.*]` entry has -- because that is the shape whose derived
//! identity used to be discarded.

use super::*;
use crate::core::config::e2e::{ArgMapping, CallOverride};

fn clear_backends_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "SampleBackend".into(),
            clear_fn: Some("clear_sample_backends".into()),
            unregister_fn: Some("unregister_sample_backend".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn clear_call(function: &str) -> CallConfig {
    CallConfig {
        function: function.into(),
        returns_result: false,
        returns_void: true,
        ..CallConfig::default()
    }
}

fn clear_fixture() -> Fixture {
    Fixture {
        id: "clear_sample_backends".into(),
        description: "Clear registered sample backends".into(),
        call: Some("clear_sample_backends".into()),
        ..Fixture::default()
    }
}

/// A populated base `function` is the Rust core's name for the operation, shared by every
/// language. It is not evidence about the C export, which `registration.rs` derives from
/// the trait name. This fails against the pre-fix code, where the derived identity was
/// applied only `if info.function_name.is_empty()`: a configured base name shadowed it and
/// the snippet called the plural `sample_clear_sample_backends`, a symbol the generated
/// header never declares.
#[test]
fn configured_base_function_does_not_shadow_the_derived_clear_symbol() {
    let mut e2e = E2eConfig::default();
    e2e.calls
        .insert("clear_sample_backends".into(), clear_call("clear_sample_backends"));

    let info = resolve_fixture_call_info(&clear_fixture(), &e2e, &clear_backends_config(), "c", CallIr::default());

    assert_eq!(info.function_name, "clear_sample_backend");
}

/// The same shadowing also swallowed the trailing `out_error` out-param that
/// `clear_fn.jinja` always declares, because both live behind the one `is_empty()` guard.
/// Pinned separately from the name so a fix that restores the symbol but drops the
/// out-param cannot pass.
#[test]
fn configured_base_function_does_not_shadow_the_clear_out_error_out_param() {
    let mut e2e = E2eConfig::default();
    e2e.calls
        .insert("clear_sample_backends".into(), clear_call("clear_sample_backends"));

    let info = resolve_fixture_call_info(&clear_fixture(), &e2e, &clear_backends_config(), "c", CallIr::default());

    assert_eq!(info.extra_args, vec!["NULL".to_string()]);
}

/// End-to-end: the rendered snippet must call the symbol the header declares, with the
/// out-param. Exercised through `render_c_snippet` so the prefixing and template layers
/// are covered too, not just the resolver.
#[test]
fn rendered_snippet_calls_the_derived_clear_symbol_with_its_out_param() {
    let mut e2e = E2eConfig::default();
    e2e.calls
        .insert("clear_sample_backends".into(), clear_call("clear_sample_backends"));

    let rendered =
        render_c_snippet(&clear_fixture(), &e2e, &clear_backends_config(), &[], &[]).expect("C snippet renders");

    assert!(rendered.contains("sample_clear_sample_backend(NULL)"), "{rendered}");
    assert!(!rendered.contains("sample_clear_sample_backends("), "{rendered}");
}

/// `unregister`'s derived name coincides with its config text, so only the out-param
/// distinguishes the two paths here: a configured base `function` must not cost the
/// snippet its second argument. Pre-fix this emitted
/// `sample_unregister_sample_backend("nonexistent-backend")` against a two-parameter ABI.
#[test]
fn configured_base_function_does_not_shadow_the_unregister_out_error_out_param() {
    let fixture = Fixture {
        id: "unregister_sample_backend".into(),
        description: "Unregister a sample backend".into(),
        call: Some("unregister_sample_backend".into()),
        input: serde_json::json!({ "name": "nonexistent-backend" }),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.calls.insert(
        "unregister_sample_backend".into(),
        CallConfig {
            function: "unregister_sample_backend".into(),
            returns_result: false,
            returns_void: true,
            args: vec![ArgMapping {
                name: "name".into(),
                field: "input.name".into(),
                arg_type: "string".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
    );

    let rendered = render_c_snippet(&fixture, &e2e, &clear_backends_config(), &[], &[]).expect("C snippet renders");

    assert!(
        rendered.contains("sample_unregister_sample_backend(\"nonexistent-backend\", NULL)"),
        "{rendered}"
    );
}

/// An explicit `[e2e.calls.*.overrides.c] function` is the one statement in the config
/// that is *about* the C export, so it keeps outranking the derivation -- a consumer whose
/// FFI layer hand-exports a differently named wrapper must still be able to say so. This
/// guards the fix from over-reaching into a config surface it has no business rewriting.
#[test]
fn an_explicit_c_function_override_still_outranks_the_derived_symbol() {
    let mut e2e = E2eConfig::default();
    let mut call = clear_call("clear_sample_backends");
    call.overrides.insert(
        "c".into(),
        CallOverride {
            function: Some("sample_drop_every_backend".into()),
            ..CallOverride::default()
        },
    );
    e2e.calls.insert("clear_sample_backends".into(), call);

    let info = resolve_fixture_call_info(&clear_fixture(), &e2e, &clear_backends_config(), "c", CallIr::default());

    assert_eq!(info.function_name, "sample_drop_every_backend");
}
