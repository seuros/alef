//! Compiled proof that the codegen pattern `structs::gen_delegating_deserialize_impl` emits --
//! `<CoreType as serde::Deserialize>::deserialize(deserializer).map(Into::into)` in place of a
//! derived, field-by-field object `Deserialize` -- actually round-trips real JSON for every
//! wire-shape class a container-level `#[serde(from/into)]` can produce.
//!
//! These modules do not call into alef's generator; they hand-write the exact pattern the
//! generator emits (mirroring `gen_from_core_to_binding_cfg`'s existing `From<Core> for
//! Binding` output plus the new delegating `Deserialize`) so the *technique* itself is proven
//! against the real `serde_json` crate, independent of any generator plumbing. Generator-level
//! assertions on the rendered text live in `src/codegen/generators/structs/tests.rs`.
#![allow(clippy::float_cmp)]

use serde::{Deserialize, Serialize};

// --- Wire-shape class 1: a two-field primitive pair -------------------------------------------

mod two_field_pair {
    use super::*;

    /// Stands in for a consumer's hand-written core type: legacy wire compatibility means it
    /// always serializes/deserializes as a positional `[x, y]` array, never a `{"x":..}` object.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(from = "(f64, f64)", into = "(f64, f64)")]
    pub struct CorePoint {
        pub x: f64,
        pub y: f64,
    }

    impl From<(f64, f64)> for CorePoint {
        fn from((x, y): (f64, f64)) -> Self {
            CorePoint { x, y }
        }
    }
    impl From<CorePoint> for (f64, f64) {
        fn from(p: CorePoint) -> Self {
            (p.x, p.y)
        }
    }

    /// Mirrors what alef's `structs::gen_struct*` emits for `Point` once it carries a
    /// `serde_container_conversion`: a plain-field DTO with `Serialize` still derived
    /// (out of scope for this fix) and a hand-written, delegating `Deserialize`.
    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct BindingPoint {
        pub x: f64,
        pub y: f64,
    }

    // Mirrors the pre-existing `gen_from_core_to_binding_cfg` / `gen_from_binding_to_core_cfg`
    // output -- unaffected by this change, included so the test can round-trip both directions.
    impl From<CorePoint> for BindingPoint {
        fn from(v: CorePoint) -> Self {
            BindingPoint { x: v.x, y: v.y }
        }
    }
    impl From<BindingPoint> for CorePoint {
        fn from(v: BindingPoint) -> Self {
            CorePoint { x: v.x, y: v.y }
        }
    }

    // This is exactly `structs/delegating_deserialize_impl.jinja`'s output for `Point`.
    impl<'de> Deserialize<'de> for BindingPoint {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <CorePoint as Deserialize>::deserialize(deserializer).map(Into::into)
        }
    }

    #[test]
    fn round_trips_both_directions() {
        let core = CorePoint { x: 1.5, y: -2.25 };

        // Sanity: the core type really does emit the legacy positional array, not an object.
        let json = serde_json::to_string(&core).unwrap();
        assert_eq!(json, "[1.5,-2.25]");

        // Direction A (core -> binding, the actual fix): decode the array into the DTO.
        let dto: BindingPoint =
            serde_json::from_str(&json).expect("delegated Deserialize must decode the core array shape");
        assert_eq!(dto, BindingPoint { x: 1.5, y: -2.25 });

        // Direction B (binding -> core, pre-existing and unaffected): convert back and compare.
        let round_tripped: CorePoint = dto.into();
        assert_eq!(round_tripped, core);
    }
}

// --- Wire-shape class 2: a four-field pair -----------------------------------------------------

mod four_field_pair {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(from = "(f64, f64, f64, f64)", into = "(f64, f64, f64, f64)")]
    pub struct CoreRect {
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
    }

    impl From<(f64, f64, f64, f64)> for CoreRect {
        fn from((x, y, w, h): (f64, f64, f64, f64)) -> Self {
            CoreRect { x, y, w, h }
        }
    }
    impl From<CoreRect> for (f64, f64, f64, f64) {
        fn from(r: CoreRect) -> Self {
            (r.x, r.y, r.w, r.h)
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct BindingRect {
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
    }

    impl From<CoreRect> for BindingRect {
        fn from(v: CoreRect) -> Self {
            BindingRect {
                x: v.x,
                y: v.y,
                w: v.w,
                h: v.h,
            }
        }
    }
    impl From<BindingRect> for CoreRect {
        fn from(v: BindingRect) -> Self {
            CoreRect {
                x: v.x,
                y: v.y,
                w: v.w,
                h: v.h,
            }
        }
    }

    impl<'de> Deserialize<'de> for BindingRect {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <CoreRect as Deserialize>::deserialize(deserializer).map(Into::into)
        }
    }

    #[test]
    fn round_trips_both_directions() {
        let core = CoreRect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 20.0,
        };
        let json = serde_json::to_string(&core).unwrap();
        assert_eq!(json, "[0.0,0.0,10.0,20.0]");

        let dto: BindingRect = serde_json::from_str(&json).expect("delegated Deserialize must decode the 4-tuple");
        assert_eq!(dto, BindingRect::from(core.clone()));

        let round_tripped: CoreRect = dto.into();
        assert_eq!(round_tripped, core);
    }
}

// --- Wire-shape class 3: a nested-struct element -----------------------------------------------

mod nested_struct_element {
    use super::two_field_pair::{BindingPoint, CorePoint};
    use super::*;

    /// `origin` is itself a container-conversion type; the outer wire tuple holds it at
    /// position 0, proving delegation composes through a nested `Deserialize` call rather than
    /// alef needing to special-case nesting.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(from = "(CorePoint, f64)", into = "(CorePoint, f64)")]
    pub struct CoreRay {
        pub origin: CorePoint,
        pub length: f64,
    }

    impl From<(CorePoint, f64)> for CoreRay {
        fn from((origin, length): (CorePoint, f64)) -> Self {
            CoreRay { origin, length }
        }
    }
    impl From<CoreRay> for (CorePoint, f64) {
        fn from(r: CoreRay) -> Self {
            (r.origin, r.length)
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct BindingRay {
        pub origin: BindingPoint,
        pub length: f64,
    }

    impl From<CoreRay> for BindingRay {
        fn from(v: CoreRay) -> Self {
            BindingRay {
                origin: v.origin.into(),
                length: v.length,
            }
        }
    }
    impl From<BindingRay> for CoreRay {
        fn from(v: BindingRay) -> Self {
            CoreRay {
                origin: v.origin.into(),
                length: v.length,
            }
        }
    }

    impl<'de> Deserialize<'de> for BindingRay {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <CoreRay as Deserialize>::deserialize(deserializer).map(Into::into)
        }
    }

    #[test]
    fn round_trips_both_directions_through_nested_delegation() {
        let core = CoreRay {
            origin: CorePoint { x: 1.0, y: 2.0 },
            length: 5.0,
        };
        let json = serde_json::to_string(&core).unwrap();
        assert_eq!(json, "[[1.0,2.0],5.0]");

        let dto: BindingRay = serde_json::from_str(&json).expect("nested delegated Deserialize must decode");
        assert_eq!(
            dto,
            BindingRay {
                origin: BindingPoint { x: 1.0, y: 2.0 },
                length: 5.0
            }
        );

        let round_tripped: CoreRay = dto.into();
        assert_eq!(round_tripped, core);
    }
}

// --- Wire-shape class 4: an optional element mid-tuple ------------------------------------------

mod optional_element_mid_tuple {
    use super::*;

    /// Variable-length positional wire: a caller-omitted `b` shortens the array to 2 elements;
    /// an explicit JSON `null` for `b` keeps a 3-element array with `None` at position 1. Both
    /// are real, distinct wire encodings a hand-written `From<Vec<Option<f64>>>` may legally
    /// choose between -- alef cannot see or guess this, which is exactly why delegation (not
    /// positional field synthesis) is the only sound fix.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(from = "Vec<Option<f64>>", into = "Vec<Option<f64>>")]
    pub struct CoreOptMid {
        pub a: f64,
        pub b: Option<f64>,
        pub c: f64,
    }

    impl From<Vec<Option<f64>>> for CoreOptMid {
        fn from(v: Vec<Option<f64>>) -> Self {
            match v.len() {
                2 => CoreOptMid {
                    a: v[0].expect("a present"),
                    b: None,
                    c: v[1].expect("c present"),
                },
                _ => CoreOptMid {
                    a: v[0].expect("a present"),
                    b: v[1],
                    c: v[2].expect("c present"),
                },
            }
        }
    }
    impl From<CoreOptMid> for Vec<Option<f64>> {
        fn from(v: CoreOptMid) -> Self {
            match v.b {
                Some(b) => vec![Some(v.a), Some(b), Some(v.c)],
                None => vec![Some(v.a), Some(v.c)],
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    pub struct BindingOptMid {
        pub a: f64,
        pub b: Option<f64>,
        pub c: f64,
    }

    impl From<CoreOptMid> for BindingOptMid {
        fn from(v: CoreOptMid) -> Self {
            BindingOptMid { a: v.a, b: v.b, c: v.c }
        }
    }
    impl From<BindingOptMid> for CoreOptMid {
        fn from(v: BindingOptMid) -> Self {
            CoreOptMid { a: v.a, b: v.b, c: v.c }
        }
    }

    impl<'de> Deserialize<'de> for BindingOptMid {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <CoreOptMid as Deserialize>::deserialize(deserializer).map(Into::into)
        }
    }

    #[test]
    fn omitted_element_round_trips_both_directions() {
        let core = CoreOptMid {
            a: 1.0,
            b: None,
            c: 3.0,
        };
        let json = serde_json::to_string(&core).unwrap();
        assert_eq!(json, "[1.0,3.0]", "omitted b shortens the array");

        let dto: BindingOptMid = serde_json::from_str(&json).expect("must decode the shortened array");
        assert_eq!(
            dto,
            BindingOptMid {
                a: 1.0,
                b: None,
                c: 3.0
            }
        );

        let round_tripped: CoreOptMid = dto.into();
        assert_eq!(round_tripped, core);
    }

    #[test]
    fn explicit_null_element_round_trips_both_directions() {
        // Simulates a producer that always emits the full-length array with an explicit null,
        // distinct from the omitted (shortened) encoding above but decoding to the same value.
        let json = "[1.0,null,3.0]";
        let dto: BindingOptMid = serde_json::from_str(json).expect("must decode the explicit null element");
        assert_eq!(
            dto,
            BindingOptMid {
                a: 1.0,
                b: None,
                c: 3.0
            }
        );

        let round_tripped: CoreOptMid = dto.into();
        assert_eq!(
            round_tripped,
            CoreOptMid {
                a: 1.0,
                b: None,
                c: 3.0
            }
        );
    }

    #[test]
    fn present_element_round_trips_both_directions() {
        let core = CoreOptMid {
            a: 1.0,
            b: Some(2.0),
            c: 3.0,
        };
        let json = serde_json::to_string(&core).unwrap();
        assert_eq!(json, "[1.0,2.0,3.0]");

        let dto: BindingOptMid = serde_json::from_str(&json).expect("must decode the full array");
        assert_eq!(
            dto,
            BindingOptMid {
                a: 1.0,
                b: Some(2.0),
                c: 3.0
            }
        );

        let round_tripped: CoreOptMid = dto.into();
        assert_eq!(round_tripped, core);
    }
}

// --- Wire-shape class 5: an enum-variant named field --------------------------------------------

mod enum_variant_named_field {
    use super::two_field_pair::{BindingPoint, CorePoint};
    use super::*;

    /// The enum itself carries no container conversion (alef's IR only tracks
    /// `serde_container_conversion` on `TypeDef`/structs) -- this class instead proves that a
    /// nested struct's delegating `Deserialize` is picked up automatically when the struct
    /// appears inside a `Vec<T>` field of an enum variant, exercised through the ordinary
    /// derive machinery `codegen::conversions::helpers::enum_arms` targets, not through any
    /// change to that module.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum CoreGeometry {
        Quad { points: Vec<CorePoint> },
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub enum BindingGeometry {
        Quad { points: Vec<BindingPoint> },
    }

    // Mirrors `enum_arms::core_to_binding_match_arm_ext_cfg` / `binding_to_core_match_arm_ext_cfg`.
    impl From<CoreGeometry> for BindingGeometry {
        fn from(v: CoreGeometry) -> Self {
            match v {
                CoreGeometry::Quad { points } => BindingGeometry::Quad {
                    points: points.into_iter().map(Into::into).collect(),
                },
            }
        }
    }
    impl From<BindingGeometry> for CoreGeometry {
        fn from(v: BindingGeometry) -> Self {
            match v {
                BindingGeometry::Quad { points } => CoreGeometry::Quad {
                    points: points.into_iter().map(Into::into).collect(),
                },
            }
        }
    }

    #[test]
    fn round_trips_both_directions_through_enum_variant_vec_field() {
        let core = CoreGeometry::Quad {
            points: vec![CorePoint { x: 1.0, y: 2.0 }, CorePoint { x: 3.0, y: 4.0 }],
        };
        let json = serde_json::to_string(&core).unwrap();
        assert_eq!(json, r#"{"Quad":{"points":[[1.0,2.0],[3.0,4.0]]}}"#);

        // Direction A: BindingGeometry's own derive decodes the enum shell; each Vec<BindingPoint>
        // element decodes through BindingPoint's delegating Deserialize -- no enum-specific code
        // was written for this to work.
        let dto: BindingGeometry = serde_json::from_str(&json).expect("nested points must decode via delegation");
        assert_eq!(
            dto,
            BindingGeometry::Quad {
                points: vec![BindingPoint { x: 1.0, y: 2.0 }, BindingPoint { x: 3.0, y: 4.0 }]
            }
        );

        // Direction B: the pre-existing enum_arms-style conversion closes the loop.
        let round_tripped: CoreGeometry = dto.into();
        assert_eq!(round_tripped, core);
    }
}
