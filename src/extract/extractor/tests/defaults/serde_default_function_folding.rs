//! Regression coverage for the extraction gap that blocked regenerating a consumer repo whose
//! source looked like:
//!
//! ```rust,ignore
//! #[derive(Debug, Clone, Default, Serialize, Deserialize)]
//! pub struct OcrElement {
//!     pub geometry: OcrBoundingGeometry,   // named type, required
//!     pub confidence: OcrConfidence,       // named type, required
//!     #[serde(default = "default_page_number")]
//!     pub page_number: u32,
//! }
//!
//! fn default_page_number() -> u32 { 1 }
//! ```
//!
//! `helpers::fields::extract_field` recorded `page_number`'s default as
//! `DefaultValue::FunctionCall("default_page_number")` unconditionally. Generation then tried to
//! recover the real value through `rust_default_via_source_deserialize` — deserializing an
//! empty-field JSON stub through `OcrElement`'s own `Deserialize` impl — which requires a JSON
//! placeholder for every other required sibling field; `json_placeholder_literal` has no
//! placeholder for a `TypeRef::Named` field, so the two required named-type siblings
//! (`geometry`, `confidence`) made that recovery impossible, and generation failed closed with
//! "cannot preserve 1 serde default function(s)". The real value was readable all along, one
//! level up: `default_page_number`'s own body is the literal `1`.
use super::*;
use crate::codegen::config_gen::validate_rust_default_functions;
use crate::core::ir::DefaultValue;

/// The exact reported shape, reduced to the minimum that reproduces the failure: two required
/// named-type sibling fields (so `rust_default_via_source_deserialize` cannot build a probe) plus
/// one `#[serde(default = "path")]` field naming a private, free, literal-returning function.
const OCR_ELEMENT_SOURCE: &str = r#"
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct OcrElement {
        pub geometry: OcrBoundingGeometry,
        pub confidence: OcrConfidence,
        #[serde(default = "default_page_number")]
        pub page_number: u32,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct OcrBoundingGeometry {
        pub x: f64,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct OcrConfidence {
        pub value: f64,
    }

    fn default_page_number() -> u32 {
        1
    }
"#;

#[test]
fn private_free_function_default_folds_to_its_literal_value() {
    let surface = extract_from_source(OCR_ELEMENT_SOURCE);
    let element = surface
        .types
        .iter()
        .find(|typ| typ.name == "OcrElement")
        .expect("OcrElement must be extracted");
    let page_number = element
        .fields
        .iter()
        .find(|field| field.name == "page_number")
        .expect("page_number field must be extracted");

    assert_eq!(
        page_number.typed_default,
        Some(DefaultValue::IntLiteral(1)),
        "default_page_number()'s body is the literal `1`; alef must read it instead of leaving \
         an unresolvable FunctionCall"
    );
}

/// The end-to-end proof: `validate_rust_default_functions` is exactly the check that made
/// generation fail with "cannot preserve 1 serde default function(s)". Once the field's default
/// is folded to a literal, that check has nothing left to refuse.
#[test]
fn ocr_element_shape_passes_rust_default_function_validation() {
    let surface = extract_from_source(OCR_ELEMENT_SOURCE);
    validate_rust_default_functions(&surface).expect(
        "a foldable #[serde(default = \"path\")] default must not require the source-deserialize \
         recovery at all, so two required named-type siblings must not block it",
    );
}

/// Control: a path-qualified associated function (`Settings::default_retry_limit`) must fold
/// exactly like a bare free function — the task's second required shape.
#[test]
fn path_qualified_associated_function_default_folds_to_its_literal_value() {
    let source = r#"
        pub struct Settings;

        impl Settings {
            fn default_retry_limit() -> u32 {
                3
            }
        }

        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct Policy {
            #[serde(default = "Settings::default_retry_limit")]
            pub retry_limit: u32,
        }
    "#;

    let surface = extract_from_source(source);
    let policy = surface.types.iter().find(|typ| typ.name == "Policy").unwrap();
    let retry_limit = policy.fields.iter().find(|field| field.name == "retry_limit").unwrap();

    assert_eq!(retry_limit.typed_default, Some(DefaultValue::IntLiteral(3)));
}

/// Control: a genuinely unfoldable `#[serde(default = "path")]` function (one whose value alef
/// cannot prove statically) must keep failing generation exactly as before, when it also has a
/// required named-type sibling that blocks the `rust_default_via_source_deserialize` recovery —
/// the same blocker that made the original `OcrElement` case fail. Folding must never become a
/// way to silence `validate_rust_default_functions` for a value alef cannot actually prove.
#[test]
fn genuinely_unfoldable_function_default_still_fails_validation() {
    let source = r#"
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct Cache {
            pub warmup: WarmupPolicy,
            #[serde(default = "computed_default")]
            pub warm_entries: u32,
        }

        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct WarmupPolicy {
            pub eager: bool,
        }

        fn computed_default() -> u32 {
            let base = read_warm_entry_count_from_disk();
            base
        }
    "#;

    let surface = extract_from_source(source);
    let cache = surface.types.iter().find(|typ| typ.name == "Cache").unwrap();
    let warm_entries = cache.fields.iter().find(|field| field.name == "warm_entries").unwrap();

    assert_eq!(
        warm_entries.typed_default,
        Some(DefaultValue::FunctionCall("computed_default".to_string())),
        "a multi-statement, non-foldable body must stay FunctionCall, not be guessed at"
    );

    let error = validate_rust_default_functions(&surface).expect_err(
        "a genuinely unfoldable serde default function, blocked from the source-deserialize \
         recovery by a required named-type sibling, must still fail generation rather than \
         silently ship a guessed value",
    );
    assert!(
        error.to_string().contains("computed_default"),
        "the failure must still name the unreadable function: {error}"
    );
}

/// Resolution must commit to the function the path actually names. `Settings::default_retry`
/// names the associated function; if that body cannot be folded, the answer is "not readable",
/// not "read whatever free function happens to share the last segment". A same-module free
/// `default_retry` is a different function with a different value, and silently substituting it
/// is the read-the-wrong-artifact failure this pass must not commit — the value would look
/// exactly as authoritative as one alef genuinely read.
#[test]
fn an_unfoldable_associated_function_never_falls_back_to_a_same_named_free_function() {
    let source = r#"
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct Settings {
            pub name: Label,
            #[serde(default = "Settings::default_retry")]
            pub retry: u32,
        }

        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct Label {
            pub text: String,
        }

        impl Settings {
            pub fn default_retry() -> u32 {
                let base = read_retry_from_env();
                base
            }
        }

        fn default_retry() -> u32 {
            99
        }
    "#;

    let surface = extract_from_source(source);
    let settings = surface.types.iter().find(|typ| typ.name == "Settings").unwrap();
    let retry = settings.fields.iter().find(|field| field.name == "retry").unwrap();

    // Not `IntLiteral(99)`: that is the free function's value, and reading it here would be the
    // whole defect. The fold declines, and `postprocess::resolve_public_default_functions` then
    // upgrades the still-unfolded path to `PublicFunctionCall` because the associated function is
    // `pub` — a real, callable answer for the Rust-emitting backends, and honestly "not folded"
    // for every other one.
    assert_eq!(
        retry.typed_default,
        Some(DefaultValue::PublicFunctionCall(
            "test_crate::Settings::default_retry".to_string()
        )),
        "the path names the associated function; when its body is unfoldable the value stays \
         unread, and the unrelated free `default_retry` (99) must never stand in for it"
    );
    assert_ne!(
        retry.typed_default,
        Some(DefaultValue::IntLiteral(99)),
        "the same-named free function's value must never be substituted for the associated one"
    );
}
