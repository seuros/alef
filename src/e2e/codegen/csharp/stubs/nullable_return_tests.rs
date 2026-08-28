//! Regression coverage for `non_null_stub_default`: a computed C# stub default of the literal
//! `"null"` must never reach a property/return whose declared type is non-nullable (no trailing
//! `?`) -- `TypeRef::Path` (`PathBuf`) maps to plain `string` and defaulted to `"null"`, which
//! compiled to `public string Method() => null;` -- CS8603, "possible null reference return".

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
use crate::e2e::fixture::Fixture;

use super::{emit_test_backend, non_null_stub_default};

/// A `"null"` default against a non-nullable `string` return must become an empty string
/// literal, not the null it was computed as -- the exact CS8603 shape this closes.
#[test]
fn a_null_default_against_a_non_nullable_string_becomes_an_empty_string() {
    assert_eq!(non_null_stub_default("null".to_string(), "string"), "\"\"");
}

/// The same computed default against a genuinely nullable `string?` return must pass through
/// unchanged -- this function must not "fix" a case that was never broken.
#[test]
fn a_null_default_against_a_nullable_type_is_left_alone() {
    assert_eq!(non_null_stub_default("null".to_string(), "string?"), "null");
}

/// A `"null"` default against a non-nullable class/record type falls back to a parameterless
/// constructor call rather than an empty string, which would not even type-check.
#[test]
fn a_null_default_against_a_non_nullable_class_type_constructs_a_default_instance() {
    assert_eq!(
        non_null_stub_default("null".to_string(), "SampleConfig"),
        "new SampleConfig()"
    );
}

/// Any non-null default (the overwhelming majority of cases) must pass through completely
/// unchanged -- this function only ever touches the exact literal `"null"`.
#[test]
fn a_non_null_default_is_never_touched() {
    assert_eq!(non_null_stub_default("1".to_string(), "long"), "1");
    assert_eq!(non_null_stub_default("\"\"".to_string(), "string"), "\"\"");
}

fn path_returning_method() -> MethodDef {
    MethodDef {
        name: "cache_dir".to_string(),
        params: vec![ParamDef {
            name: "hint".to_string(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Path,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: Some("OcrBackend".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// End-to-end companion to the unit tests above, through the real stub emitter: a trait method
/// returning `PathBuf` must never emit `=> null;`.
#[test]
fn a_path_returning_trait_method_stub_never_returns_a_bare_null() {
    let method = path_returning_method();
    let bridge = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "register_ocr_backend".to_string(),
        description: "Register an OCR backend".to_string(),
        input: serde_json::json!({ "name": "sample-ocr" }),
        ..Fixture::default()
    };

    let emission = emit_test_backend(&bridge, &[&method], &fixture);

    assert!(
        !emission.setup_block.contains("=> null;"),
        "a non-nullable `string` return must never be `=> null;` (CS8603): {}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("public string CacheDir(string hint)"),
        "{}",
        emission.setup_block
    );
    assert!(emission.setup_block.contains("=> \"\";"), "{}", emission.setup_block);
}
