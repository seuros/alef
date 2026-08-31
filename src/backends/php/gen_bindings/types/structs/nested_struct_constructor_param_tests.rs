//! `php_field_can_be_constructor_param`'s `Named` arm: a bare nested `#[php_class]` struct field
//! (neither an enum, an opaque type, nor an untagged data enum) is now a real constructor
//! parameter — taken by reference, per the SAME `&{ClassName}` shape `gen_php_function_params`
//! already renders for every other Named-struct function/method parameter — instead of being
//! silently defaulted or refused. Each positive case here is paired with a negative control using
//! the exact type that predicate still excludes, so a regression that widened (or narrowed) the
//! arm past its intended boundary fails one half of the pair.

use super::*;
use crate::backends::php::type_map::PhpMapper;

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

#[test]
fn plain_nested_struct_is_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Named("Result".to_string()),
        &AHashSet::new(),
        &AHashSet::new(),
        &AHashSet::new(),
    ));
}

#[test]
fn opaque_named_type_stays_unrepresentable() {
    assert!(!php_field_can_be_constructor_param(
        &TypeRef::Named("Client".to_string()),
        &AHashSet::new(),
        &names(&["Client"]),
        &AHashSet::new(),
    ));
}

/// Weaker sibling of the test below: only proves an enum-named field is representable at
/// all (which both the widened `Named` arm and the `is_php_prop_scalar_with_enums` fallback
/// answer identically -- `is_php_prop_scalar_with_enums`'s own `Named(n) if
/// enum_names.contains(n) => true` arm already returns `true` for this exact input, so this
/// assertion cannot tell which arm fired). Kept as a basic sanity/regression guard against
/// this function refusing an enum-named field outright; the arm-specific guarantee is the
/// next test.
#[test]
fn enum_named_type_is_representable() {
    assert!(php_field_can_be_constructor_param(
        &TypeRef::Named("Mode".to_string()),
        &names(&["Mode"]),
        &AHashSet::new(),
        &AHashSet::new(),
    ));
}

/// The test the boolean above cannot be: it asserts on the OBSERVABLE behaviour that
/// distinguishes the widened `Named` arm from the `is_php_prop_scalar_with_enums` fallback --
/// whether the constructor takes the field BY REFERENCE and clones it. That decision is made
/// by `is_named_struct_by_ref` (`constructor_init.rs`), a function with its OWN independent
/// `!enum_names.contains(...)` guard; `php_field_can_be_constructor_param`'s guard, by
/// contrast, does not change this function's return value for an enum field at all (deleting
/// it just makes the SAME `true` come from a different arm) -- so this test necessarily
/// exercises the by-ref/clone boundary end to end rather than the standalone predicate.
/// A required nested struct field (`result`) forces `has_named_params` (independent of this
/// widening) to true, routing generation through the per-field-filtered constructor path;
/// the all-prop-scalar plain-constructor path never consults `php_field_can_be_constructor_param`
/// at all and would not exercise anything here.
#[test]
fn enum_named_required_field_uses_the_plain_shorthand_not_a_reference_clone() {
    let typ = TypeDef {
        name: "ModeHolder".to_string(),
        rust_path: "test_lib::ModeHolder".to_string(),
        fields: vec![
            field("path", TypeRef::String, false),
            field("result", TypeRef::Named("Outcome".to_string()), false),
            field("mode", TypeRef::Named("Mode".to_string()), false),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let mapper = PhpMapper {
        enum_names: names(&["Mode"]),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper,
        true,
        "test_lib",
        &AHashSet::new(),
        &names(&["Mode"]),
        &[],
        &[],
        &AHashSet::new(),
        &[],
        &AHashSet::new(),
        &[],
    )
    .expect("struct methods generate");

    let new_fn = out
        .split("#[php(constructor)]")
        .nth(1)
        .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));
    // Scope assertions to the constructor's OWN body -- `new_fn` is everything AFTER the
    // `#[php(constructor)]` marker, which also includes every subsequent method (e.g. the
    // `get_mode` getter, which legitimately does `self.mode.clone()`). Checking the whole
    // tail would let a getter's unrelated `.clone()` satisfy or defeat an assertion meant for
    // the constructor body alone.
    let ctor_only = new_fn
        .split("\n\n")
        .next()
        .unwrap_or_else(|| panic!("constructor body has no blank-line terminator:\n{new_fn}"));

    assert!(
        ctor_only.contains("mode: String") && !ctor_only.contains("mode: &"),
        "an enum-named param must be an owned String, never a reference, got:\n{ctor_only}"
    );
    assert!(
        ctor_only.contains("mode }"),
        "the enum field must use the plain shorthand initialiser as the last field, got:\n{ctor_only}"
    );
    assert!(
        !ctor_only.contains("mode.clone()") && !ctor_only.contains("mode: mode"),
        "an enum-named field must never be cloned out of a reference, got:\n{ctor_only}"
    );
}

#[test]
fn untagged_data_enum_named_type_stays_unrepresentable() {
    assert!(!php_field_can_be_constructor_param(
        &TypeRef::Named("Payload".to_string()),
        &AHashSet::new(),
        &AHashSet::new(),
        &names(&["Payload"]),
    ));
}

fn names(values: &[&str]) -> AHashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn mapper() -> PhpMapper {
    PhpMapper {
        enum_names: AHashSet::new(),
        data_enum_names: AHashSet::new(),
        untagged_data_enum_names: AHashSet::new(),
        json_string_enum_names: AHashSet::new(),
    }
}

/// End-to-end regression pin: a struct with a required nested `#[php_class]` field emits a
/// `#[php(constructor)]` that takes the nested type BY REFERENCE and stores an owned CLONE of
/// it, not a `Default::default()` placeholder and not `.map(|v| v.clone())` (which trips
/// `clippy::map_clone` in the consumer crate this generated code lands in).
#[test]
fn required_nested_struct_field_is_taken_by_reference_and_cloned_into_self() {
    let typ = TypeDef {
        name: "ArchiveEntry".to_string(),
        rust_path: "test_lib::ArchiveEntry".to_string(),
        fields: vec![
            field("path", TypeRef::String, false),
            field("result", TypeRef::Named("Outcome".to_string()), false),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper(),
        true,
        "test_lib",
        &AHashSet::new(),
        &AHashSet::new(),
        &[],
        &[],
        &AHashSet::new(),
        &[],
        &AHashSet::new(),
        &[],
    )
    .expect("struct methods generate");

    let new_fn = out
        .split("#[php(constructor)]")
        .nth(1)
        .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));

    assert!(
        new_fn.contains("result: &Outcome"),
        "a required nested struct field must be a by-reference constructor param, got:\n{new_fn}"
    );
    assert!(
        new_fn.contains("result: result.clone()"),
        "the field must be initialised from a clone of the reference, got:\n{new_fn}"
    );
    assert!(
        !new_fn.contains("result: Default::default()"),
        "a representable field must never be defaulted away, got:\n{new_fn}"
    );
}

/// Same pin as above for the `Option<T>`-shaped case: `.cloned()` on `Option<&T>`, never the
/// longer `.map(|v| v.clone())` form that means the same thing but trips `clippy::map_clone`.
#[test]
fn optional_nested_struct_field_uses_cloned_not_map_clone() {
    let typ = TypeDef {
        name: "ArchiveEntry".to_string(),
        rust_path: "test_lib::ArchiveEntry".to_string(),
        fields: vec![
            field("path", TypeRef::String, false),
            field("result", TypeRef::Named("Outcome".to_string()), true),
        ],
        has_default: false,
        has_serde: true,
        ..Default::default()
    };

    let out = gen_struct_methods_with_exclude(
        &typ,
        &mapper(),
        true,
        "test_lib",
        &AHashSet::new(),
        &AHashSet::new(),
        &[],
        &[],
        &AHashSet::new(),
        &[],
        &AHashSet::new(),
        &[],
    )
    .expect("struct methods generate");

    let new_fn = out
        .split("#[php(constructor)]")
        .nth(1)
        .unwrap_or_else(|| panic!("no #[php(constructor)] fn emitted:\n{out}"));

    assert!(
        new_fn.contains("result: Option<&Outcome>"),
        "an optional nested struct field must be Option<&T>, got:\n{new_fn}"
    );
    assert!(
        new_fn.contains("result: result.cloned()"),
        "must emit `.cloned()`, not `.map(|v| v.clone())`, got:\n{new_fn}"
    );
    assert!(!new_fn.contains("map(|v| v.clone())"), "got:\n{new_fn}");
}
