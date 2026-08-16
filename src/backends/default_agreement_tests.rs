//! The cross-language control for field defaults.
//!
//! Every backend reads a field's default off the same [`FieldDef::typed_default`], so two
//! backends rendering one IR fixture must land on the same value. A backend that quietly stops
//! consuming that field does not produce an error — it produces its target language's *zero*,
//! which is a perfectly plausible-looking initializer. Only a comparison across backends
//! distinguishes "the default was rendered" from "the default was dropped and the zero took its
//! place". Both live defects this control was written for were exactly that shape: C# rendered
//! `false` where Rust said `true`, and Java rendered nothing where Rust said `0.7`.
//!
//! [`crate::backends::swift::gen_bindings::dto::swift_typed_default_literal`] is the oracle. It
//! is suffix-free, so a disagreement it reports is a disagreement about the *value* rather than
//! about spelling.
//!
//! Three things are pinned here, and the split matters:
//!
//! 1. [`carries_per_field_default`] classifies **every** [`Language`], with an exhaustive match
//!    and no wildcard arm. A new backend does not compile until someone states how it carries a
//!    Rust default. This is the anti-drift half: an exclusion that is merely *implied* by a test
//!    that never mentions a backend is how a backend leaves the control unnoticed.
//! 2. The shared [`default_value_for_field`] renderer — which go, magnus, php, wasm and extendr
//!    all reach — must agree with the oracle for every language it serves.
//! 3. Per-field literal agreement for the backends that emit an initializer directly into a
//!    struct/record body lives next to each emitter, because their renderers are module-private:
//!    C# in `backends::csharp::gen_bindings::types::tests`, Java in
//!    `backends::java::gen_bindings::types::tests`, Kotlin in
//!    `backends::kotlin::gen_bindings::object_wrapper::tests`. Each compares against the same
//!    oracle, so all four agree transitively.

use crate::backends::swift::gen_bindings::dto::swift_typed_default_literal;
use crate::codegen::config_gen::default_value_for_field;
use crate::core::config::Language;
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeRef};

/// How a language's binding carries a Rust field default to the core crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultCarrier {
    /// The emitted struct/record/class spells the default as a per-field initializer or default
    /// argument. A dropped variant here becomes the target language's zero and crosses the FFI.
    PerFieldLiteral,
    /// The binding never spells the value. The unset field is absent from the payload handed to
    /// Rust (or the binding seeds itself from the core type's own `Default`), so Rust supplies
    /// the real default. Dropping a variant here costs ergonomics, never correctness.
    DelegatesToRust,
    /// Not a binding target: no user-facing struct is emitted at all.
    NoUserFacingStruct,
}

/// The classification. Deliberately an exhaustive match over [`Language`] with no `_` arm: adding
/// a language is then a compile error until its default-carrying behaviour is stated here. ~keep
const fn carries_per_field_default(language: Language) -> DefaultCarrier {
    match language {
        // Emit an initializer into the type body; audited against the oracle next to each emitter.
        Language::Csharp | Language::Java | Language::Kotlin | Language::KotlinAndroid | Language::Swift => {
            DefaultCarrier::PerFieldLiteral
        }
        // Emit a default argument or a synthesized constructor carrying the literal.
        Language::Python | Language::Dart | Language::Ruby | Language::Elixir | Language::Go | Language::Wasm => {
            DefaultCarrier::PerFieldLiteral
        }
        // Node/napi widens every defaulted field to `Option<T>` and drives binding->core in
        // builder mode seeded from `CoreType::default()`, so an unset property leaves the real
        // Rust value in place. R/extendr does the same with `<T>::default()` plus `NULL` args.
        // PHP gives the binding struct a `Default` impl that delegates to the core type's and
        // puts `#[serde(default)]` on the container, so a missing key routes through it.
        Language::Node | Language::R | Language::Php => DefaultCarrier::DelegatesToRust,
        // Gleam has no default record fields or default arguments in the language. Zig emits
        // bare `name: T,` and is a decode target for Rust-produced JSON, so a Zig-side
        // initializer would never travel back.
        Language::Gleam | Language::Zig => DefaultCarrier::DelegatesToRust,
        // FFI is the C ABI (opaque handles plus `_from_json`/`_to_json`); JNI emits the Rust
        // `Java_*` shim crate, not the Java surface. Rust and C are docs/e2e targets with no
        // backend at all.
        Language::Ffi | Language::Jni | Language::Rust | Language::C => DefaultCarrier::NoUserFacingStruct,
    }
}

fn scalar_field(ty: TypeRef, value: DefaultValue) -> FieldDef {
    FieldDef {
        name: "fixture".to_string(),
        ty,
        typed_default: Some(value),
        ..Default::default()
    }
}

/// The fixture: one bool, one float with a fractional part, one whole-valued float, one integer
/// too large to be confused with an index, and one string. Every one of them has a target-language
/// zero that differs from the Rust default, which is the whole point — a value equal to the zero
/// cannot distinguish "rendered" from "dropped".
fn agreement_fixture() -> Vec<(&'static str, FieldDef)> {
    vec![
        (
            "bool true",
            scalar_field(TypeRef::Primitive(PrimitiveType::Bool), DefaultValue::BoolLiteral(true)),
        ),
        (
            "fractional float",
            scalar_field(TypeRef::Primitive(PrimitiveType::F32), DefaultValue::FloatLiteral(0.7)),
        ),
        (
            "whole-valued float",
            scalar_field(TypeRef::Primitive(PrimitiveType::F64), DefaultValue::FloatLiteral(2.0)),
        ),
        (
            "large integer",
            scalar_field(
                TypeRef::Primitive(PrimitiveType::U64),
                DefaultValue::IntLiteral(10_485_760),
            ),
        ),
        (
            "string",
            scalar_field(TypeRef::String, DefaultValue::StringLiteral("balanced".to_string())),
        ),
    ]
}

/// Every language the shared renderer serves. `default_value_for_field` dispatches on these
/// strings; a language it does not recognise silently takes a fallback arm, so naming them
/// explicitly is what keeps this loop from testing one arm five times.
const SHARED_RENDERER_LANGUAGES: [&str; 8] = ["python", "ruby", "go", "java", "csharp", "php", "r", "rust"];

/// Strip the spellings a language adds around a value that the oracle does not, leaving only the
/// value itself. Normalising is deliberate: excluding a language because it writes `True` instead
/// of `true` would let it drift out of the comparison entirely.
fn normalize_literal(rendered: &str) -> String {
    rendered.trim_end_matches(".to_string()").to_lowercase()
}

#[test]
fn every_language_states_how_it_carries_a_rust_default() {
    for language in Language::ALL {
        let carrier = carries_per_field_default(language);
        if crate::cli::registry::try_get_backend(language).is_none() {
            assert_eq!(
                carrier,
                DefaultCarrier::NoUserFacingStruct,
                "{language} has no backend, so it cannot be classified as emitting anything"
            );
        }
    }
}

/// The apparatus check. If the oracle stopped rendering these variants, every agreement assertion
/// below would compare `None` against `None` and pass while examining nothing.
#[test]
fn the_oracle_renders_every_fixture_value() {
    for (label, field) in agreement_fixture() {
        let value = field.typed_default.as_ref().expect("fixture field carries a default");
        assert!(
            swift_typed_default_literal(value).is_some(),
            "the oracle must render `{label}` or it cannot referee anything"
        );
    }
}

/// The control, at the shared renderer. go, magnus, php, wasm and extendr all resolve a field
/// default through `default_value_for_field`; if it and the oracle disagree, one of them has
/// dropped or mangled the value.
#[test]
fn the_shared_renderer_agrees_with_the_oracle_for_every_language_it_serves() {
    for (label, field) in agreement_fixture() {
        let value = field.typed_default.as_ref().expect("fixture field carries a default");
        let expected = normalize_literal(&swift_typed_default_literal(value).expect("oracle renders the fixture"));

        for language in SHARED_RENDERER_LANGUAGES {
            let rendered = normalize_literal(&default_value_for_field(&field, language));
            assert_eq!(
                rendered, expected,
                "`{language}` and the Swift oracle disagree on the default for `{label}`"
            );
        }
    }
}

/// `EnumVariant` is the one fixture value the oracle cannot referee — Swift returns `None` for it,
/// and every language spells a variant differently (`Mode.FAST`, `Mode::Fast`, `ModeFast`,
/// `"fast"`). Comparing spellings would be meaningless, but leaving the variant out of the control
/// altogether is how a backend starts dropping it unnoticed. So the assertion is the weaker true
/// one: every language must render *something* that names the variant.
#[test]
fn every_language_renders_an_enum_variant_default_as_something() {
    let field = FieldDef {
        name: "mode".to_string(),
        ty: TypeRef::Named("Mode".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Fast".to_string())),
        ..Default::default()
    };

    for language in SHARED_RENDERER_LANGUAGES {
        let rendered = default_value_for_field(&field, language);
        assert!(
            rendered.to_lowercase().contains("fast"),
            "`{language}` dropped the enum-variant default, rendering `{rendered}`"
        );
        assert!(
            rendered.contains("Mode"),
            "`{language}` rendered the variant without its enum, giving `{rendered}`"
        );
    }
}

/// The negative control for [`normalize_literal`]. Without it, a normaliser that collapsed
/// everything to the empty string would make every agreement assertion above pass.
#[test]
fn the_normalizer_does_not_erase_a_real_disagreement() {
    assert_ne!(normalize_literal("0.7"), normalize_literal("0.0"));
    assert_ne!(normalize_literal("true"), normalize_literal("false"));
    assert_ne!(
        normalize_literal("\"balanced\".to_string()"),
        normalize_literal("\"\".to_string()")
    );
    assert_eq!(
        normalize_literal("\"balanced\".to_string()"),
        normalize_literal("\"balanced\"")
    );
    assert_eq!(normalize_literal("True"), normalize_literal("true"));
}
