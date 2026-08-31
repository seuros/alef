//! Every test here is paired with its opposite control. The failure this module exists to
//! prevent is silent — a fabricated empty allow-list compiles, runs, and passes every smoke
//! test — so a suite that only exercised the refusal would stay green if generation started
//! failing for every input, and a suite that only exercised the happy path would stay green if
//! the refusal were deleted. The pairs below pin both directions. ~keep

use super::*;
use crate::core::ir::PrimitiveType;

const CORE_TYPE: &str = "sample_core::FetchPolicy";

fn names(values: &[&str]) -> AHashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn field(name: &str, ty: TypeRef, typed_default: Option<DefaultValue>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        typed_default,
        ..Default::default()
    }
}

/// A `Vec<SomeStruct>` whose inner type is neither opaque nor an enum: not representable as a
/// PHP constructor parameter, so the constructor omits it. This is the allow-list shape.
fn rule_list() -> TypeRef {
    TypeRef::Vec(Box::new(TypeRef::Named("Rule".to_string())))
}

/// A single nested struct field — the scalar (non-collection) half of the shape axis. Also not
/// representable, for the same reason.
fn nested_policy() -> TypeRef {
    TypeRef::Named("SsrfPolicy".to_string())
}

fn policy(has_default: bool, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: "FetchPolicy".to_string(),
        rust_path: CORE_TYPE.to_string(),
        has_default,
        has_serde: true,
        fields,
        ..Default::default()
    }
}

fn build(typ: &TypeDef) -> anyhow::Result<ConstructorInit> {
    gen_constructor_field_inits(typ, &names(&["Mode"]), &names(&["Client"]))
}

// ---------------------------------------------------------------------------
// Axis 1: derived `Default`. Every field the derive covers is `DefaultValue::Empty`, which the
// IR defines as "the default IS this type's own zero" — so the target-language zero is exact,
// not a guess, and must still be emitted.
// ---------------------------------------------------------------------------

#[test]
fn should_emit_type_zero_when_derived_default_marks_collection_field_empty() {
    let typ = policy(true, vec![field("allow_list", rule_list(), Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("a derived Default is knowable and must not fail generation");

    assert_eq!(init.field_inits, "allow_list: Default::default()");
    assert_eq!(
        init.prelude, "",
        "an exact type zero needs no delegating-Default local"
    );
}

#[test]
fn should_emit_type_zero_when_derived_default_marks_scalar_field_empty() {
    let typ = policy(true, vec![field("ssrf", nested_policy(), Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("a derived Default is knowable and must not fail generation");

    assert_eq!(init.field_inits, "ssrf: Default::default()");
}

/// The `Empty` reading is a property of the recorded default, not of the owning type: a field
/// carrying a bare `#[serde(default)]` on a type with no `Default` impl is still exactly the
/// type zero. Without this arm the refusal below would swallow a case it has no business
/// refusing.
#[test]
fn should_emit_type_zero_for_empty_default_even_without_a_type_level_default_impl() {
    let typ = policy(false, vec![field("allow_list", rule_list(), Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("an explicitly-empty default is knowable regardless of the owning type");

    assert_eq!(init.field_inits, "allow_list: Default::default()");
}

// ---------------------------------------------------------------------------
// Axis 2: manual `impl Default`. The value is not a type zero, and for an unreadable body alef
// cannot spell it at all — but the impl is real, compiled Rust, so the binding reads it back
// off its own delegating `Default` rather than guessing.
// ---------------------------------------------------------------------------

#[test]
fn should_read_collection_field_off_core_default_when_manual_impl_is_unresolved() {
    let typ = policy(
        true,
        vec![field(
            "allow_list",
            rule_list(),
            Some(DefaultValue::Unresolved("Self::builder().build()".to_string())),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "allow_list: __alef_core_defaults.allow_list");
    assert_eq!(
        init.prelude, "let __alef_core_defaults = <Self as ::core::default::Default>::default();\n",
        "the recovery must bind the delegating Default exactly once, before `Self {{ .. }}`"
    );
    assert!(
        !init.field_inits.contains("Default::default()"),
        "an unresolved manual Default must never fall back to the type zero, got: {}",
        init.field_inits
    );
}

#[test]
fn should_read_collection_field_off_core_default_when_manual_impl_holds_a_list_literal() {
    let typ = policy(
        true,
        vec![field(
            "allow_list",
            rule_list(),
            Some(DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral(
                "internal".to_string(),
            )])),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "allow_list: __alef_core_defaults.allow_list");
}

#[test]
fn should_read_scalar_field_off_core_default_when_manual_impl_names_an_enum_variant() {
    let typ = policy(
        true,
        vec![field(
            "ssrf",
            nested_policy(),
            Some(DefaultValue::EnumVariant("Strict".to_string())),
        )],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(init.field_inits, "ssrf: __alef_core_defaults.ssrf");
}

/// Two recovered fields must share one local. Binding it per field would move the same value
/// twice; binding it never would leave the initialisers referring to nothing.
#[test]
fn should_bind_the_core_default_local_once_for_several_recovered_fields() {
    let typ = policy(
        true,
        vec![
            field("allow_list", rule_list(), Some(DefaultValue::Unresolved("x".to_string()))),
            field("deny_list", rule_list(), Some(DefaultValue::Unresolved("x".to_string()))),
        ],
    );

    let init = build(&typ).expect("a type with a Default impl can always be read back from");

    assert_eq!(
        init.field_inits,
        "allow_list: __alef_core_defaults.allow_list, deny_list: __alef_core_defaults.deny_list"
    );
    assert_eq!(init.prelude.matches("let __alef_core_defaults").count(), 1);
}

// ---------------------------------------------------------------------------
// Axis 3: no `Default` at all. There is no real value to recover, so generation fails rather
// than inventing one.
//
// Security direction, stated explicitly because the two lists fail in OPPOSITE directions and
// both directions are wrong:
//   - an allow-list fabricated empty admits nothing legitimately, so callers relax it or the
//     check is skipped — it fails OPEN in practice, and in the common "empty means unrestricted"
//     reading it fails open outright;
//   - a deny-list fabricated empty blocks nothing — it fails OPEN unambiguously.
// Neither is observable at runtime: the PHP caller cannot distinguish a value the Rust author
// chose from one alef invented. So the refusal is symmetric and does not try to guess a "safe"
// direction per field. ~keep
// ---------------------------------------------------------------------------

#[test]
fn should_fail_generation_for_an_allow_list_with_no_recoverable_default() {
    let typ = policy(false, vec![field("allow_list", rule_list(), None)]);

    let error = build(&typ).expect_err("an unknowable allow-list default must fail generation");
    let message = format!("{error}");

    assert!(message.contains("allow_list"), "error must name the field, got: {message}");
    assert!(message.contains(CORE_TYPE), "error must name the type, got: {message}");
    // Not merely `contains("Default")` — the message mentions `Default::default()` while
    // explaining what it refuses to emit, so that would pass with the remedy list deleted. ~keep
    assert!(
        message.contains("#[derive(Default)]"),
        "error must name the remedy, got: {message}"
    );
}

#[test]
fn should_fail_generation_for_a_deny_list_with_no_recoverable_default() {
    let typ = policy(false, vec![field("deny_list", rule_list(), None)]);

    let error = build(&typ).expect_err("an unknowable deny-list default must fail generation");

    assert!(format!("{error}").contains("deny_list"));
}

#[test]
fn should_fail_generation_for_an_unknowable_scalar_field() {
    let typ = policy(false, vec![field("ssrf", nested_policy(), None)]);

    let error = build(&typ).expect_err("an unknowable nested-struct default must fail generation");

    assert!(format!("{error}").contains("ssrf"));
}

/// The refusal must be scoped to fields the constructor omits. A representable field with no
/// recorded default is handed to the caller as a parameter, so nothing is invented and nothing
/// should fail — this is the control that catches a refusal widened past its purpose.
#[test]
fn should_not_fail_when_the_field_with_no_default_is_a_real_constructor_parameter() {
    let typ = policy(
        false,
        vec![field("max_depth", TypeRef::Primitive(PrimitiveType::U32), None)],
    );

    let init = build(&typ).expect("a representable field takes its value from the caller");

    assert_eq!(init.field_inits, "max_depth: maxDepth");
}

/// Ordering control: one unknowable field must fail the whole constructor even when a knowable
/// sibling would otherwise render. A per-field fallback that "skipped" the bad field would pass
/// every test above and still ship the fabrication.
#[test]
fn should_fail_when_any_omitted_field_is_unknowable_even_beside_a_knowable_one() {
    let typ = policy(
        false,
        vec![
            field("allow_list", rule_list(), Some(DefaultValue::Empty)),
            field("deny_list", rule_list(), None),
        ],
    );

    let error = build(&typ).expect_err("one unknowable field must fail the constructor");

    assert!(format!("{error}").contains("deny_list"));
}

/// `None` for an omitted `Option` field is not the fabrication the refusal targets: it encodes
/// "absent", which is exactly true of a field the caller had no way to pass, and no core
/// `Default` can be contradicting it because this shape requires the type to have none.
#[test]
fn should_emit_none_for_an_omitted_option_field_with_no_type_level_default() {
    let typ = policy(
        false,
        vec![field("ssrf", TypeRef::Optional(Box::new(nested_policy())), None)],
    );

    let init = build(&typ).expect("an omitted Option field is honestly absent, not invented");

    assert_eq!(init.field_inits, "ssrf: None");
}

/// The discrimination control for the arm above: wrapping a *collection* in `Option` is the only
/// thing that makes it acceptable. A bare collection with the same provenance must still fail, or
/// the `Option` arm has widened into the blanket fallback this module exists to delete.
#[test]
fn should_still_fail_for_a_bare_collection_beside_an_acceptable_option_field() {
    let typ = policy(
        false,
        vec![
            field("ssrf", TypeRef::Optional(Box::new(nested_policy())), None),
            field("allow_list", rule_list(), None),
        ],
    );

    let error = build(&typ).expect_err("an Option sibling must not license a bare collection");

    assert!(format!("{error}").contains("allow_list"));
}

// ---------------------------------------------------------------------------
// Untouched paths.
// ---------------------------------------------------------------------------

#[test]
fn should_keep_binding_excluded_fields_out_of_the_initialiser_entirely() {
    let mut excluded = field("allow_list", rule_list(), None);
    excluded.binding_excluded = true;
    let typ = policy(
        false,
        vec![
            excluded,
            field("max_depth", TypeRef::Primitive(PrimitiveType::U32), None),
        ],
    );

    let init = build(&typ).expect("an excluded field is not part of the binding struct");

    assert_eq!(init.field_inits, "max_depth: maxDepth");
}

#[test]
fn should_unwrap_the_bytes_newtype_for_a_representable_bytes_field() {
    let typ = policy(true, vec![field("payload", TypeRef::Bytes, Some(DefaultValue::Empty))]);

    let init = build(&typ).expect("Bytes is a representable constructor parameter");

    assert_eq!(init.field_inits, "payload: payload.0");
}
