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
use crate::codegen::shared::config_constructor_parts_with_options;
use crate::core::config::Language;
use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};

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

/// The axis this control cannot referee by comparison, and what referees it instead.
///
/// Everything above compares a [`DefaultValue`] that *carries a value*. [`DefaultValue::Empty`],
/// [`DefaultValue::Unresolved`] and the function-call variants carry none, the oracle correctly
/// returns `None` for all of them, and no backend can therefore be compared against anything on
/// that path. Every backend nevertheless renders *something* there: its target language's
/// type-zero.
///
/// That substitution used to be licensed by nothing checkable. `Empty` meant "the default is
/// exactly the type-zero" when it came from `#[derive(Default)]` and "the extractor could not read
/// it" when it came from a manual `impl Default` — two opposite claims carried by one variant, and
/// no backend distinguished them. The flag one would reach for,
/// [`crate::core::ir::TypeDef::has_default`], cannot separate them either: `impl_blocks` sets it
/// for a manual impl too, so it is `true` in both cases.
///
/// [`DefaultValue::Unresolved`] now carries the distinction in the value itself, where a backend
/// cannot look past it, and `cli::pipeline::generate::validation` refuses to generate rather than
/// let a zero stand in for a default alef never read. ~keep
#[test]
fn a_default_that_carries_no_value_is_declined_by_the_oracle() {
    for unreadable in [
        DefaultValue::Empty,
        DefaultValue::Unresolved("Self::new(\"en\")".to_string()),
        DefaultValue::FunctionCall("crate::defaults::limit".to_string()),
        DefaultValue::PublicFunctionCall("crate::defaults::limit".to_string()),
    ] {
        assert!(
            swift_typed_default_literal(&unreadable).is_none(),
            "`{unreadable:?}` carries no value, so the oracle must decline to referee it"
        );
    }
}

/// The control for the disambiguation itself.
///
/// `Empty` and `Unresolved` must not be interchangeable *as values*, because the whole defect was
/// that they were one value. A refactor that folded `Unresolved` back into `Empty` — an alias, a
/// `From`, a normalising helper — would restore the conflation while every rendering assertion in
/// this module kept passing, since none of them can see the difference.
#[test]
fn empty_and_unresolved_are_distinct_values() {
    assert_ne!(
        DefaultValue::Empty,
        DefaultValue::Unresolved(String::new()),
        "`Empty` asserts the default IS the type-zero; `Unresolved` asserts alef does not know it"
    );
    assert!(
        matches!(DefaultValue::Unresolved("body".to_string()), DefaultValue::Unresolved(body) if body == "body"),
        "`Unresolved` must retain the unreadable body so the diagnostic can name it"
    );
}

/// The meaning that must survive the disambiguation, asserted separately from the one introduced.
///
/// `#[derive(Default)]` really does give every field its type's zero, so rendering a zero there is
/// exact, not a guess. If the `Unresolved` remedy were applied to `Empty` as well — by making the
/// shared renderer refuse it, or emit no initializer — every derived-`Default` type in every
/// binding would lose an initializer it is entitled to. This pins the zero.
#[test]
fn a_derived_default_still_renders_as_the_target_language_zero() {
    let bool_field = scalar_field(TypeRef::Primitive(PrimitiveType::Bool), DefaultValue::Empty);
    for (language, expected) in [("python", "False"), ("go", "false"), ("csharp", "false")] {
        assert_eq!(
            default_value_for_field(&bool_field, language),
            expected,
            "a `#[derive(Default)]` bool field must still render `{language}`'s zero"
        );
    }

    let int_field = scalar_field(TypeRef::Primitive(PrimitiveType::U64), DefaultValue::Empty);
    for (language, expected) in [("python", "0"), ("go", "0"), ("csharp", "0")] {
        assert_eq!(
            default_value_for_field(&int_field, language),
            expected,
            "a `#[derive(Default)]` integer field must still render `{language}`'s zero"
        );
    }
}

/// The apparatus check for the pair above, in the spirit of
/// [`the_normalizer_does_not_erase_a_real_disagreement`]: an `Unresolved` value must actually
/// survive into the shared renderer. If some earlier layer quietly rewrote it back to `Empty`,
/// both assertions would keep passing while refereeing a value that no longer exists.
#[test]
fn the_shared_renderer_receives_unresolved_without_it_being_rewritten() {
    let field = scalar_field(
        TypeRef::Primitive(PrimitiveType::F32),
        DefaultValue::Unresolved("Self::new(\"en\")".to_string()),
    );
    assert!(
        matches!(field.typed_default, Some(DefaultValue::Unresolved(_))),
        "the fixture itself must carry `Unresolved`, or this referees nothing"
    );
    // Rendering is not the refusal point — `cli::pipeline::generate::validation` is, and it runs
    // first. What this pins is that the renderer stays *reachable* with the variant, so the
    // documented `suppress_validation_codes` escape hatch has defined behaviour, not a panic. ~keep
    for language in SHARED_RENDERER_LANGUAGES {
        let rendered = default_value_for_field(&field, language);
        assert!(
            !rendered.is_empty(),
            "`{language}` must produce a defined rendering for a suppressed `Unresolved`"
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

// ---------------------------------------------------------------------------------------------
// The second default class: `#[serde(default = "path")]`.
//
// Everything above referees a `DefaultValue` that carries a value, so backends can be compared
// against each other. `FunctionCall`/`PublicFunctionCall` carry only the *name* of a Rust
// function, so there is nothing to compare — which is exactly how this class survived the round
// that fixed the literals. The assertions below are therefore about *absence*: whatever a backend
// emits for one of these, it must not be the value it would have emitted had the field carried no
// default at all. That comparison is the only one that distinguishes "the default was carried" or
// "the default was declined" from "the default was dropped and the type's zero took its place".
// ---------------------------------------------------------------------------------------------

/// A field carrying a private `#[serde(default = "path")]`, spelled the way the extractor spells
/// it: the marker in `default` and the callee path in `typed_default`
/// (`extract::extractor::helpers::fields`). Both halves matter — a renderer that consults only
/// `default` and one that consults only `typed_default` fail differently.
fn function_call_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        default: Some("serde(default = \"default_stop_words\")".to_string()),
        typed_default: Some(DefaultValue::FunctionCall("default_stop_words".to_string())),
        ..Default::default()
    }
}

/// The same field with the default stripped: what a backend renders when it has dropped the
/// default entirely. This is the discriminator, not a fixture value, because the *value* of a
/// function-call default is by construction unavailable to compare against.
fn same_field_without_any_default(field: &FieldDef) -> FieldDef {
    FieldDef {
        default: None,
        typed_default: None,
        ..field.clone()
    }
}

/// The apparatus check for the two tests below. If the no-default rendering were empty (or equal
/// across every shape), `assert_ne!` against it would pass while examining nothing — the same
/// vacuous-pass failure `the_normalizer_does_not_erase_a_real_disagreement` guards for the
/// literal half.
#[test]
fn the_no_default_rendering_is_a_real_value_for_every_language_it_is_compared_against() {
    for ty in [
        TypeRef::Primitive(PrimitiveType::U64),
        TypeRef::Vec(Box::new(TypeRef::String)),
    ] {
        let bare = same_field_without_any_default(&function_call_field("stop_words", ty.clone()));
        for language in SHARED_RENDERER_LANGUAGES {
            let zero = default_value_for_field(&bare, language);
            assert!(
                !zero.trim().is_empty(),
                "`{language}` renders nothing for a bare `{ty:?}`, so comparing against it proves nothing"
            );
        }
    }
}

/// The control for the function-call class at the shared renderer (go, magnus, php, wasm and
/// extendr reach it). A `#[serde(default = "path")]` field must not be answered with the value a
/// field of the same type would get with no default at all: `0` for the integer, `[]`/`nil` for
/// the collection. Both fixture shapes have a target-language zero that is a perfectly plausible
/// initializer, which is what made the drop invisible.
#[test]
fn a_function_call_default_is_never_answered_with_the_no_default_value() {
    for ty in [
        TypeRef::Primitive(PrimitiveType::U64),
        TypeRef::Vec(Box::new(TypeRef::String)),
    ] {
        let field = function_call_field("stop_words", ty.clone());
        let bare = same_field_without_any_default(&field);
        for language in SHARED_RENDERER_LANGUAGES {
            let rendered = default_value_for_field(&field, language);
            let zero = default_value_for_field(&bare, language);
            assert_ne!(
                rendered, zero,
                "`{language}` answered a `#[serde(default = \"...\")]` on `{ty:?}` with the same `{zero}` a field \
                 with no default gets — the default was dropped, not carried"
            );
        }
    }
}

/// An owning `TypeDef` for the Rust-emitting constructor generators (wasm, pyo3, extendr all
/// reach `config_constructor_parts_*`). `has_default` is what licenses reading a field's real
/// value back off `<CoreType as Default>::default()`.
fn owning_type_with_default(rust_path: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "Settings".to_string(),
        rust_path: rust_path.to_string(),
        has_default: true,
        has_serde: true,
        fields,
        ..Default::default()
    }
}

/// A binding-side type mapper shaped like wasm's: a `Named` field becomes a distinct wrapper
/// type, which is what makes the `.into()` on a recovered core value load-bearing.
fn wasm_style_mapper() -> impl Fn(&TypeRef) -> String {
    |ty: &TypeRef| match ty {
        TypeRef::Named(name) => format!("Wasm{name}"),
        _ => "String".to_string(),
    }
}

/// `EnumVariant` is the other value the extractor lowers lossily: `SomeEnum::Variant` keeps only
/// `"Variant"`, so there is no path a Rust-emitting backend can spell. The old answer was
/// `unwrap_or_default()`, which calls the *field type's* `Default` — a different value from the
/// variant the field defaults to whenever the two disagree, and the disagreement is invisible
/// because both are valid values of the same enum. The owning type's own `Default` is the one
/// place that knows, so the value is read back off it.
#[test]
fn an_enum_variant_default_is_read_off_the_owning_types_default() {
    let field = FieldDef {
        name: "mode".to_string(),
        ty: TypeRef::Named("Mode".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Fast".to_string())),
        ..Default::default()
    };
    let typ = owning_type_with_default("demo::Settings", vec![field.clone()]);

    let (_, _, assignments) = config_constructor_parts_with_options(&[field], &wasm_style_mapper(), false, &typ);

    assert!(
        assignments.contains("<demo::Settings as ::core::default::Default>::default().mode"),
        "the variant must be read off the owning type, not guessed: {assignments}"
    );
    assert!(
        !assignments.contains("mode.unwrap_or_default()"),
        "`unwrap_or_default()` calls `Mode::default()`, which is not necessarily `Mode::Fast`: {assignments}"
    );
    assert!(
        assignments.contains(".into()"),
        "the recovered value is the core enum and the binding field is the wrapper: {assignments}"
    );
}

/// The negative control for the test above. The recovery is only available when alef can name the
/// owning type; without a `rust_path` there is no expression to emit and `unwrap_or_default()` is
/// all that remains. Without this, the assertion above could be passing because *every* input
/// produces the recovery, which would say nothing about the enum case in particular.
#[test]
fn an_enum_variant_default_falls_back_only_when_the_owning_type_cannot_be_named() {
    let field = FieldDef {
        name: "mode".to_string(),
        ty: TypeRef::Named("Mode".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("Fast".to_string())),
        ..Default::default()
    };
    let typ = owning_type_with_default("", vec![field.clone()]);

    let (_, _, assignments) = config_constructor_parts_with_options(&[field], &wasm_style_mapper(), false, &typ);

    assert!(
        assignments.contains("mode.unwrap_or_default()"),
        "an unnameable owning type leaves nothing to read the variant off: {assignments}"
    );
}

/// `Empty` must keep its `unwrap_or_default()` rendering: it *means* `Default::default()`, so the
/// binding type's own default is exact. Pinned next to the `EnumVariant` change because the two
/// shared one match arm until this round — widening the recovery to `Empty` as well would replace
/// an exact answer with a needlessly indirect one.
#[test]
fn an_empty_default_still_delegates_to_the_binding_types_own_default() {
    let field = FieldDef {
        name: "tags".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        typed_default: Some(DefaultValue::Empty),
        ..Default::default()
    };
    let typ = owning_type_with_default("demo::Settings", vec![field.clone()]);

    let (_, _, assignments) = config_constructor_parts_with_options(&[field], &wasm_style_mapper(), false, &typ);

    assert!(
        assignments.contains("tags.unwrap_or_default()"),
        "`Empty` is exactly `Default::default()` and needs no recovery: {assignments}"
    );
}
