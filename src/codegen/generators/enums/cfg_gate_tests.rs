use crate::codegen::generators::{gen_pyo3_data_enum, gen_pyo3_data_enum_with_mapper};
use crate::codegen::type_mapper::IdentityMapper;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn named_tuple_variant(name: &str, inner_type: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named(inner_type.to_string()),
            ..Default::default()
        }],
        is_tuple: true,
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn struct_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "text".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn data_enum(rust_path: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "VisitorResult".to_string(),
        rust_path: rust_path.to_string(),
        variants,
        has_serde: true,
        serde_tag: Some("type".to_string()),
        ..Default::default()
    }
}

/// The regression this task fixes: `write_pyo3_variant_accessors`'s fast-path `#[getter]` for a
/// single-Named-tuple-field variant matches `core_path::Variant(data) => Some(...)` directly,
/// unconditionally, regardless of `EnumVariant::cfg` -- E0599 in a build excluding the variant's
/// feature. A host-owned cfg-gated variant must keep its arm, gated with `#[cfg(...)]`.
#[test]
fn host_owned_cfg_variant_keeps_accessor_arm_and_gate() {
    let def = data_enum(
        "mylib::VisitorResult",
        vec![
            unit_variant("Continue", None),
            named_tuple_variant("Thumbnail", "ThumbnailData", Some(r#"feature = "thumbnails""#)),
        ],
    );
    let generated = gen_pyo3_data_enum(&def, "mylib");

    assert!(
        generated.contains("mylib::VisitorResult::Thumbnail(data)"),
        "the host-owned variant's accessor arm must still be emitted, got:\n{generated}"
    );
    assert_eq!(
        generated.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the host-owned variant's accessor arm must carry its #[cfg] guard exactly once, got:\n{generated}"
    );
}

/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate. Forwarding it as `#[cfg(...)]` names a feature this pyo3 crate never declares -- an
/// `unexpected cfg condition value` warning -- so the accessor arm must be dropped entirely
/// instead, mirroring `codegen::conversions::enums::emit_cfg_gated_arm`. The `_ => None` fallback
/// already covers the dropped case.
#[test]
fn foreign_owned_cfg_variant_accessor_arm_is_dropped_not_gated() {
    let def = data_enum(
        "dep_crate::VisitorResult",
        vec![
            unit_variant("Continue", None),
            named_tuple_variant("Testkit", "TestkitData", Some(r#"feature = "testkit""#)),
        ],
    );
    let generated = gen_pyo3_data_enum(&def, "mylib");

    assert!(
        !generated.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{generated}"
    );
    assert!(
        !generated.contains("::Testkit(data)"),
        "a foreign-crate cfg-gated variant must not be referenced, got:\n{generated}"
    );
    assert!(
        generated.contains("_ => None,"),
        "dropping the arm must still leave the match exhaustive via the fallback, got:\n{generated}"
    );
}

/// Negative control: an ungated data enum emits no `#[cfg(...)]` at all in its accessors.
#[test]
fn ungated_data_enum_emits_no_cfg_in_accessors() {
    let def = data_enum(
        "mylib::VisitorResult",
        vec![
            unit_variant("Continue", None),
            named_tuple_variant("Thumbnail", "ThumbnailData", None),
        ],
    );
    let generated = gen_pyo3_data_enum(&def, "mylib");

    assert!(
        !generated.contains("#[cfg("),
        "ungated enum must not emit #[cfg(...)], got:\n{generated}"
    );
}

/// The second regression this task fixes: `gen_pyo3_enum_variant_constructors_content` builds an
/// entire `#[staticmethod]` factory function per struct-shaped variant
/// (`Self { inner: core_path::Variant { .. } }`), unconditionally referencing the cfg-gated
/// variant -- E0599 in a build excluding its feature. A host-owned cfg-gated variant's factory
/// must be kept, with the whole function gated by `#[cfg(...)]`.
#[test]
fn host_owned_cfg_variant_keeps_its_factory_and_gate() {
    let def = data_enum(
        "mylib::VisitorResult",
        vec![
            unit_variant("Continue", None),
            struct_variant("Thumbnail", Some(r#"feature = "thumbnails""#)),
        ],
    );
    let generated = gen_pyo3_data_enum_with_mapper(&def, "mylib", Some(&IdentityMapper));

    assert!(
        generated.contains("_factory_thumbnail"),
        "the host-owned variant's factory must still be emitted, got:\n{generated}"
    );
    assert_eq!(
        generated.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the host-owned variant's factory must carry its #[cfg] guard exactly once, got:\n{generated}"
    );
}

/// A foreign-owned cfg-gated struct variant's factory constructor must be dropped entirely --
/// there is no match arm to gate around, the whole function exists only to construct that one
/// variant, so an invalid #[cfg] naming an undeclared feature must never be emitted.
#[test]
fn foreign_owned_cfg_variant_factory_is_dropped_entirely() {
    let def = data_enum(
        "dep_crate::VisitorResult",
        vec![
            unit_variant("Continue", None),
            struct_variant("Testkit", Some(r#"feature = "testkit""#)),
        ],
    );
    let generated = gen_pyo3_data_enum_with_mapper(&def, "mylib", Some(&IdentityMapper));

    assert!(
        !generated.contains("_factory_testkit"),
        "a foreign-crate cfg-gated variant's factory must not be emitted, got:\n{generated}"
    );
    assert!(
        !generated.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{generated}"
    );
}

/// Negative control: an ungated struct variant's factory carries no `#[cfg(...)]`.
#[test]
fn ungated_struct_variant_factory_emits_no_cfg() {
    let def = data_enum(
        "mylib::VisitorResult",
        vec![unit_variant("Continue", None), struct_variant("Thumbnail", None)],
    );
    let generated = gen_pyo3_data_enum_with_mapper(&def, "mylib", Some(&IdentityMapper));

    assert!(
        generated.contains("_factory_thumbnail"),
        "the ungated variant's factory must be emitted, got:\n{generated}"
    );
    assert!(!generated.contains("#[cfg("), "ungated variant must not emit #[cfg(...)], got:\n{generated}");
}
