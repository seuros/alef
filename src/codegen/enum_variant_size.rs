//! Approximate per-variant byte-size estimates for backends that emit real Rust `enum { .. }`
//! data enums, used to decide whether a generated enum should carry a narrow
//! `#[expect(clippy::large_enum_variant, reason = "...")]`.
//!
//! alef has no access to `rustc`'s actual type layout (padding, alignment, niche optimization,
//! field reordering, enum discriminant packing) at generation time — it only has the IR. Every
//! size below is therefore a coarse approximation from field *shapes*, not a measurement.
//!
//! The bias is deliberate and asymmetric: `#[expect(...)]` is a hard compile error
//! (`unfulfilled_lint_expectation`) when the lint it names does not actually fire, whereas an
//! enum this estimator under-flags simply keeps hitting the ORIGINAL `clippy::large_enum_variant`
//! error unchanged — visible, and fixable by widening the heuristic later. Every approximation
//! choice here (fallback sizes, the `Option<T>` estimate, the trust margin in
//! `enum_should_expect_large_variant_lint`) is picked to round estimates DOWN, so this estimator
//! trades false negatives (an enum that should get `#[expect]` but doesn't, leaving the original
//! clippy error in place) for avoiding false positives (an `#[expect]` emitted where the lint
//! would not have fired, hard-erroring on `unfulfilled_lint_expectation`) (alef #545). ~keep

use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};
use ahash::AHashSet;

/// Stack size (bytes) assumed for `String` / `Vec<_>` / `PathBuf`-as-`String` / JSON-as-`String`
/// on a 64-bit target: pointer + length + capacity, three machine words. Every generated field
/// type this estimator treats as a container (see [`type_ref_size_estimate`]) collapses to this
/// one number regardless of what it contains, matching how `Vec<T>`/`String` are actually sized
/// on the stack no matter how large `T` or the string content is.
const CONTAINER_SIZE_ESTIMATE_BYTES: u64 = 24;

/// Stack size (bytes) assumed for `std::collections::HashMap<_, _>` on a 64-bit target.
const MAP_SIZE_ESTIMATE_BYTES: u64 = 48;

/// Fallback estimate for a `Named` field type this estimator cannot resolve to a struct or enum
/// in [`ApiSurface`] — an external/opaque/generic type — or one it stopped resolving because of
/// the cycle/depth guards in [`named_type_size_estimate`].
///
/// Deliberately small (pointer-ish) rather than a worst-case guess, per the module-level bias:
/// undercounting an unresolved leaf risks a missed `#[expect]` (safe: the original lint still
/// fires normally), while overcounting it risks an `#[expect]` where the real lint never fires
/// (a hard `unfulfilled_lint_expectation` error). ~keep
const UNKNOWN_NAMED_TYPE_SIZE_ESTIMATE_BYTES: u64 = 8;

/// Recursion depth cap for resolving a `Named` field into its own struct/enum definition and
/// summing *its* fields in turn. Bounds both pathological IR (an accidental cycle the
/// `visiting` guard in [`named_type_size_estimate`] does not by itself rule out across siblings)
/// and ordinary deep nesting; beyond it a `Named` leaf falls back to
/// [`UNKNOWN_NAMED_TYPE_SIZE_ESTIMATE_BYTES`].
const MAX_RESOLUTION_DEPTH: usize = 6;

/// clippy's own default `enum-variant-size-threshold` (bytes): the gap between the largest and
/// second-largest variant `clippy::large_enum_variant` fires above.
const CLIPPY_DEFAULT_THRESHOLD_BYTES: u64 = 200;

/// Extra margin required on top of [`CLIPPY_DEFAULT_THRESHOLD_BYTES`] before this estimator
/// trusts its own numbers enough to assert `#[expect(...)]`.
///
/// A gap that only barely clears clippy's real threshold is exactly where the effects this
/// estimator cannot see (padding, alignment, niche optimization, field reordering) could flip
/// the real verdict either way. Requiring a wider margin trades recall — some genuinely-large
/// enums whose true gap sits between [`CLIPPY_DEFAULT_THRESHOLD_BYTES`] and
/// [`EXPECT_GAP_THRESHOLD_BYTES`] still hit the bare `clippy::large_enum_variant` error,
/// unhandled by this heuristic — for precision (an emitted `#[expect]` should essentially never
/// turn into an `unfulfilled_lint_expectation` error). The latter is the failure mode task #545
/// calls out as the one to get right. ~keep
const ESTIMATE_TRUST_MARGIN_BYTES: u64 = 200;

/// The gap an enum's two heaviest variants must clear, by this estimator's numbers, before
/// [`enum_should_expect_large_variant_lint`] returns `true`. See
/// [`ESTIMATE_TRUST_MARGIN_BYTES`] for why this is wider than clippy's own threshold.
const EXPECT_GAP_THRESHOLD_BYTES: u64 = CLIPPY_DEFAULT_THRESHOLD_BYTES + ESTIMATE_TRUST_MARGIN_BYTES;

fn primitive_size_estimate(primitive: &PrimitiveType) -> u64 {
    match primitive {
        PrimitiveType::Bool | PrimitiveType::U8 | PrimitiveType::I8 => 1,
        PrimitiveType::U16 | PrimitiveType::I16 => 2,
        PrimitiveType::U32 | PrimitiveType::I32 | PrimitiveType::F32 => 4,
        PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::F64 | PrimitiveType::Usize | PrimitiveType::Isize => 8,
    }
}

/// Estimate one `TypeRef`'s stack footprint. `visiting` carries the set of `Named` types
/// currently being resolved higher up the call chain, so a cycle collapses to the unknown-type
/// fallback instead of recursing forever; `depth` is the sibling depth counter enforcing
/// [`MAX_RESOLUTION_DEPTH`].
fn type_ref_size_estimate(ty: &TypeRef, api: &ApiSurface, visiting: &mut AHashSet<String>, depth: usize) -> u64 {
    match ty {
        TypeRef::Primitive(p) => primitive_size_estimate(p),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Bytes => {
            CONTAINER_SIZE_ESTIMATE_BYTES
        }
        TypeRef::Duration => 8,
        TypeRef::Unit => 0,
        TypeRef::Vec(_) => CONTAINER_SIZE_ESTIMATE_BYTES,
        TypeRef::Map(_, _) => MAP_SIZE_ESTIMATE_BYTES,
        // Real `Option<T>` is `size_of::<T>()` when T has a spare niche (references, `Box`,
        // `Vec`, `String`, ...) and `size_of::<T>() + padding` otherwise. Not modeling the
        // padding case undercounts `Option<primitive>` by a handful of bytes -- an
        // intentional, bounded instance of the module's round-down bias. ~keep
        TypeRef::Optional(inner) => type_ref_size_estimate(inner, api, visiting, depth),
        TypeRef::Named(name) => named_type_size_estimate(name, api, visiting, depth),
    }
}

fn named_type_size_estimate(name: &str, api: &ApiSurface, visiting: &mut AHashSet<String>, depth: usize) -> u64 {
    if depth >= MAX_RESOLUTION_DEPTH || !visiting.insert(name.to_string()) {
        return UNKNOWN_NAMED_TYPE_SIZE_ESTIMATE_BYTES;
    }

    let size = if let Some(type_def) = api.types.iter().find(|t| t.name == name) {
        type_def
            .fields
            .iter()
            .map(|field| field_size_estimate(field, api, visiting, depth + 1))
            .sum()
    } else if let Some(enum_def) = api.enums.iter().find(|e| e.name == name) {
        // A Rust enum's own size is (approximately) its heaviest variant's size plus a
        // discriminant -- reuse the same per-variant summation this module already does for
        // the enum under test, one level down.
        enum_def
            .variants
            .iter()
            .map(|variant| sum_variant_fields(variant, api, visiting, depth + 1))
            .max()
            .unwrap_or(0)
    } else {
        UNKNOWN_NAMED_TYPE_SIZE_ESTIMATE_BYTES
    };

    visiting.remove(name);
    size
}

fn field_size_estimate(field: &FieldDef, api: &ApiSurface, visiting: &mut AHashSet<String>, depth: usize) -> u64 {
    type_ref_size_estimate(&field.ty, api, visiting, depth)
}

fn sum_variant_fields(variant: &EnumVariant, api: &ApiSurface, visiting: &mut AHashSet<String>, depth: usize) -> u64 {
    variant
        .fields
        .iter()
        .map(|field| field_size_estimate(field, api, visiting, depth))
        .sum()
}

/// Estimate the in-memory size (bytes) of one enum variant, as the sum of its fields' estimated
/// sizes. See the module doc for what this can and cannot see.
fn variant_size_estimate(variant: &EnumVariant, api: &ApiSurface) -> u64 {
    let mut visiting = AHashSet::new();
    sum_variant_fields(variant, api, &mut visiting, 0)
}

/// Whether `enum_def`'s generated Rust definition should carry a narrow
/// `#[expect(clippy::large_enum_variant, reason = "...")]`.
///
/// Mirrors clippy's own check at a coarse grain: `true` when the heaviest variant's estimated
/// size exceeds the second-heaviest by more than [`EXPECT_GAP_THRESHOLD_BYTES`]. Returns
/// `false` for enums with fewer than two variants (clippy cannot flag those — there is no
/// "second largest" to compare against) and for enums whose variants are all the same
/// estimated size (unit enums score every variant at 0 bytes).
///
/// Callers must gate this to enums that actually emit as a real Rust `enum` with data-carrying
/// variants; it is not meaningful for flat-struct or unit-enum lowerings, which cannot trip
/// `clippy::large_enum_variant` in the first place.
pub fn enum_should_expect_large_variant_lint(enum_def: &EnumDef, api: &ApiSurface) -> bool {
    if enum_def.variants.len() < 2 {
        return false;
    }
    let mut sizes: Vec<u64> = enum_def
        .variants
        .iter()
        .map(|v| variant_size_estimate(v, api))
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes[0].saturating_sub(sizes[1]) > EXPECT_GAP_THRESHOLD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeDef;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            is_tuple: false,
            ..EnumVariant::default()
        }
    }

    fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("sample_crate::{name}"),
            variants,
            ..EnumDef::default()
        }
    }

    // --- table-driven `type_ref_size_estimate` cases --------------------------------------------

    #[test]
    fn type_ref_size_estimate_table() {
        let api = ApiSurface::default();
        let cases: &[(&str, TypeRef, u64)] = &[
            ("bool", TypeRef::Primitive(PrimitiveType::Bool), 1),
            ("u8", TypeRef::Primitive(PrimitiveType::U8), 1),
            ("u16", TypeRef::Primitive(PrimitiveType::U16), 2),
            ("u32", TypeRef::Primitive(PrimitiveType::U32), 4),
            ("f32", TypeRef::Primitive(PrimitiveType::F32), 4),
            ("u64", TypeRef::Primitive(PrimitiveType::U64), 8),
            ("f64", TypeRef::Primitive(PrimitiveType::F64), 8),
            ("usize", TypeRef::Primitive(PrimitiveType::Usize), 8),
            ("string", TypeRef::String, 24),
            ("bytes", TypeRef::Bytes, 24),
            ("path", TypeRef::Path, 24),
            ("json", TypeRef::Json, 24),
            ("duration", TypeRef::Duration, 8),
            ("unit", TypeRef::Unit, 0),
            (
                "vec_of_named",
                TypeRef::Vec(Box::new(TypeRef::Named("Huge".into()))),
                24,
            ),
            (
                "map",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                48,
            ),
            (
                "optional_u64",
                TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                8,
            ),
            ("unresolved_named", TypeRef::Named("NotInSurface".into()), 8),
        ];

        for (label, ty, expected) in cases {
            let mut visiting = AHashSet::new();
            let actual = type_ref_size_estimate(ty, &api, &mut visiting, 0);
            assert_eq!(actual, *expected, "case `{label}`: expected {expected}, got {actual}");
        }
    }

    // --- Named struct resolution ------------------------------------------------------------

    #[test]
    fn named_struct_recurses_into_its_own_fields() {
        let mut api = ApiSurface::default();
        api.types.push(TypeDef {
            name: "Heavy".to_string(),
            fields: vec![
                field("a", TypeRef::String),
                field("b", TypeRef::String),
                field("c", TypeRef::Primitive(PrimitiveType::U64)),
            ],
            ..TypeDef::default()
        });

        let mut visiting = AHashSet::new();
        let size = type_ref_size_estimate(&TypeRef::Named("Heavy".into()), &api, &mut visiting, 0);
        assert_eq!(size, 24 + 24 + 8, "two Strings plus one u64");
    }

    #[test]
    fn named_cycle_falls_back_to_unknown_estimate_instead_of_recursing_forever() {
        let mut api = ApiSurface::default();
        // `Wrapper` contains itself by value in the IR -- illegal in real compiled Rust
        // without indirection, but the estimator must not stack-overflow on malformed input.
        api.types.push(TypeDef {
            name: "Wrapper".to_string(),
            fields: vec![field("inner", TypeRef::Named("Wrapper".into()))],
            ..TypeDef::default()
        });

        let mut visiting = AHashSet::new();
        let size = type_ref_size_estimate(&TypeRef::Named("Wrapper".into()), &api, &mut visiting, 0);
        assert_eq!(size, UNKNOWN_NAMED_TYPE_SIZE_ESTIMATE_BYTES);
    }

    #[test]
    fn named_enum_resolves_to_its_heaviest_variant() {
        let mut api = ApiSurface::default();
        api.enums.push(enum_def(
            "Inner",
            vec![
                variant("Small", vec![field("n", TypeRef::Primitive(PrimitiveType::U8))]),
                variant("Big", vec![field("s1", TypeRef::String), field("s2", TypeRef::String)]),
            ],
        ));

        let mut visiting = AHashSet::new();
        let size = type_ref_size_estimate(&TypeRef::Named("Inner".into()), &api, &mut visiting, 0);
        assert_eq!(size, 48, "heaviest variant (two Strings) wins, not the lightest");
    }

    // --- `enum_should_expect_large_variant_lint`: the positive and negative controls ------------

    /// The reported shape: one variant wraps a `Named` struct with several `String` fields,
    /// siblings are small tuple variants. Must get `#[expect]`.
    #[test]
    fn flags_enum_whose_named_payload_dwarfs_its_siblings() {
        let mut api = ApiSurface::default();
        api.types.push(TypeDef {
            name: "HeavyConfig".to_string(),
            // 20 String fields => 20 * 24 = 480 bytes, comfortably past the 4-byte "Light"
            // sibling and past EXPECT_GAP_THRESHOLD_BYTES (400).
            fields: (0..20)
                .map(|i| field(&format!("setting_{i}"), TypeRef::String))
                .collect(),
            ..TypeDef::default()
        });
        let target = enum_def(
            "ModelKind",
            vec![
                variant("Heavy", vec![field("heavy", TypeRef::Named("HeavyConfig".into()))]),
                variant("Light", vec![field("n", TypeRef::Primitive(PrimitiveType::U32))]),
                variant("Plain", vec![]),
            ],
        );

        assert!(
            enum_should_expect_large_variant_lint(&target, &api),
            "a variant estimated at 480 bytes against 4-byte and 0-byte siblings must be flagged"
        );
    }

    /// Negative control: every variant is roughly the same estimated size. Must NOT get
    /// `#[expect]` -- this is exactly the shape that would trip `unfulfilled_lint_expectation`
    /// if the attribute were emitted unconditionally.
    #[test]
    fn does_not_flag_enum_with_similarly_sized_variants() {
        let api = ApiSurface::default();
        let target = enum_def(
            "RequestKind",
            vec![
                variant("Get", vec![field("path", TypeRef::String)]),
                variant(
                    "Post",
                    vec![field("path", TypeRef::String), field("body", TypeRef::String)],
                ),
                variant("Delete", vec![field("path", TypeRef::String)]),
            ],
        );

        assert!(
            !enum_should_expect_large_variant_lint(&target, &api),
            "variants within ~24 bytes of each other must not be flagged"
        );
    }

    /// A gap that clears clippy's own 200-byte threshold but not this estimator's wider trust
    /// margin: must NOT be flagged, by design (see [`ESTIMATE_TRUST_MARGIN_BYTES`]).
    #[test]
    fn does_not_flag_gap_inside_the_trust_margin() {
        let api = ApiSurface::default();
        let target = enum_def(
            "Borderline",
            vec![
                // 9 Strings ~= 216 bytes vs 0 for the unit sibling: clears clippy's 200-byte
                // default but not this module's 400-byte trust margin.
                variant(
                    "NineStrings",
                    (0..9).map(|i| field(&format!("f{i}"), TypeRef::String)).collect(),
                ),
                variant("Unit", vec![]),
            ],
        );

        assert!(!enum_should_expect_large_variant_lint(&target, &api));
    }

    #[test]
    fn does_not_flag_single_variant_enum() {
        let api = ApiSurface::default();
        let target = enum_def("Solo", vec![variant("Only", vec![field("s", TypeRef::String)])]);
        assert!(!enum_should_expect_large_variant_lint(&target, &api));
    }

    #[test]
    fn does_not_flag_unit_enum() {
        let api = ApiSurface::default();
        let target = enum_def("Color", vec![variant("Red", vec![]), variant("Blue", vec![])]);
        assert!(!enum_should_expect_large_variant_lint(&target, &api));
    }
}
