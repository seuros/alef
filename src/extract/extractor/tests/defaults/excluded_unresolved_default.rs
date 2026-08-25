//! Regression coverage for two related fixes to `impl Default` resolution:
//!
//! 1. `defaults::tail_is_bare_self` — a `fn default() -> Self { Self::new() }` delegating to a
//!    `new()` whose own body is just the bare `Self` path (the shape every zero-field "internal
//!    extractor" unit struct in a real consumer crate takes) is now resolved with zero per-field
//!    defaults instead of falling to the "neither a struct literal nor a foldable delegation"
//!    warning. This helps every zero-field type, excluded or not.
//! 2. `defaults::extract_default_values`'s `binding_excluded` parameter — a type genuinely
//!    excluded from every binding surface (`#[alef(skip)]` in any recognized spelling, or
//!    `#[doc(hidden)]`) no longer logs the "field defaults are unresolved" warning for a body
//!    that stays genuinely unfoldable even after fix 1, since that type never reaches codegen and
//!    the warning is pure regen-log noise. A non-excluded type with the identical unfoldable body
//!    must still warn — the diagnostic is suppressed only for items that were already dropped.
use super::*;
use tracing_test::traced_test;

/// (1) An excluded zero-field type whose manual `impl Default` delegates to `Self::new()`, whose
/// own body is bare `Self` — exactly the shape every `OdtExtractor`/`CodeExtractor`-style
/// zero-field plugin struct in a real consumer crate takes. Must resolve with no warning at all,
/// and the type must genuinely be `binding_excluded` (proving the exclusion attribute itself is
/// recognized, not merely assumed).
#[test]
#[traced_test]
fn excluded_unit_struct_delegating_to_bare_self_new_is_not_reported() {
    let source = r#"
        #[cfg_attr(alef, alef(skip))]
        pub struct OdtExtractor;

        impl OdtExtractor {
            pub(crate) fn new() -> Self {
                Self
            }
        }

        impl Default for OdtExtractor {
            fn default() -> Self {
                Self::new()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let extractor = surface
        .types
        .iter()
        .find(|typ| typ.name == "OdtExtractor")
        .expect("OdtExtractor must still be extracted (into the IR) even though it is excluded");

    assert!(
        extractor.binding_excluded,
        "#[cfg_attr(alef, alef(skip))] must set binding_excluded"
    );
    assert!(
        extractor.fields.is_empty(),
        "a unit struct has no fields to carry a default"
    );

    assert!(
        !logs_contain("unresolved"),
        "a zero-field type's Self::new() -> Self delegation is fully known (there are no \
         fields to be wrong about); it must never have been reported as unresolved, excluded or \
         not"
    );
}

/// (1) continued: the same `Self::new() -> Self` shape on a type that is *not* excluded must
/// resolve identically — the fix is a genuine improvement to what alef can read, not merely a
/// suppression that happens to also help excluded types.
#[test]
#[traced_test]
fn non_excluded_unit_struct_delegating_to_bare_self_new_is_not_reported() {
    let source = r#"
        pub struct CodeExtractor;

        impl CodeExtractor {
            pub fn new() -> Self {
                Self
            }
        }

        impl Default for CodeExtractor {
            fn default() -> Self {
                Self::new()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let extractor = surface
        .types
        .iter()
        .find(|typ| typ.name == "CodeExtractor")
        .expect("CodeExtractor must be extracted");
    assert!(!extractor.binding_excluded);

    assert!(
        !logs_contain("unresolved"),
        "the bare-Self delegation must resolve cleanly regardless of whether the type is excluded"
    );
}

/// (2) An excluded type whose `impl Default` body genuinely cannot be folded even after fix 1
/// (real computation, not a struct literal or a bare-Self/foldable delegation) must not warn: the
/// type never reaches any binding, so the warning has no actionable audience. Its field must
/// still carry the honest `Unresolved` value rather than a fabricated one.
#[test]
#[traced_test]
fn excluded_type_with_a_genuinely_unfoldable_default_is_not_reported() {
    let source = r#"
        #[alef(skip)]
        pub struct WarmedCache {
            pub capacity: usize,
        }

        impl Default for WarmedCache {
            fn default() -> Self {
                build_warmed_cache()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let cache = surface
        .types
        .iter()
        .find(|typ| typ.name == "WarmedCache")
        .expect("WarmedCache must be extracted");
    assert!(cache.binding_excluded, "bare #[alef(skip)] must set binding_excluded");

    let capacity = cache
        .fields
        .iter()
        .find(|field| field.name == "capacity")
        .expect("capacity field must be extracted");
    assert!(
        matches!(
            capacity.typed_default,
            Some(crate::core::ir::DefaultValue::Unresolved(_))
        ),
        "the field must still carry the honest Unresolved marker, not a guessed value: {:?}",
        capacity.typed_default
    );

    assert!(
        !logs_contain("unresolved"),
        "an excluded type's genuinely unfoldable default must not be logged; it never reaches a \
         binding, so the warning is pure noise"
    );
}

/// (2) continued: the identical unfoldable body on a *non*-excluded type must still warn — the
/// suppression must be scoped to `binding_excluded` items only, never a blanket silence.
#[test]
#[traced_test]
fn non_excluded_type_with_the_same_unfoldable_default_still_warns() {
    let source = r#"
        pub struct WarmedCache {
            pub capacity: usize,
        }

        impl Default for WarmedCache {
            fn default() -> Self {
                build_warmed_cache()
            }
        }
    "#;

    let surface = extract_from_source(source);
    let cache = surface
        .types
        .iter()
        .find(|typ| typ.name == "WarmedCache")
        .expect("WarmedCache must be extracted");
    assert!(!cache.binding_excluded);

    assert!(
        logs_contain("unresolved"),
        "a non-excluded type's genuinely unfoldable default is still actionable (it reaches a \
         binding) and must still be reported"
    );
}
