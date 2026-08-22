//! Guard: the C# e2e sealed-union `Display` helper must emit the same wire strings as the
//! production C# enum backend's `json_name` (`backends/csharp/gen_bindings/enums.rs:44-47`).
//!
//! Before this fix, `render_sealed_display` computed its per-variant display string with a
//! hardcoded `.to_lowercase()` of `serde_rename.unwrap_or(variant_name)` — ignoring
//! `serde_rename_all` entirely and, worse, lowercasing an explicit `#[serde(rename = "...")]`
//! that the binding preserves verbatim. That is only correct for single-word variants under
//! lowercase/snake_case, which is why it survived. `enums.rs:44-47`'s `json_name` is:
//! `v.serde_rename.clone().unwrap_or_else(|| wire_variant_value(&v.name, None, rename_all))` —
//! provably the same computation as `naming::wire_variant_value(name, serde_rename, rename_all)`
//! (the `unwrap_or_else` closure only runs when `serde_rename` is `None`, so passing `None`
//! inside it is equivalent to `serde_rename.as_deref()`). `enums.rs` is `pub(super)` inside
//! `backends::csharp::gen_bindings` and not reachable from e2e test code, so this test asserts
//! against `wire_variant_value` directly — the single seam both `enums.rs` and the fixed
//! `render_sealed_display` route through.

use crate::codegen::naming::wire_variant_value;
use crate::core::ir::{EnumDef, EnumVariant};

use super::values::render_sealed_display;

fn variant(name: &str, serde_rename: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        serde_rename: serde_rename.map(str::to_string),
        ..EnumVariant::default()
    }
}

/// `ElementBased` and `Image` are subject to `serde_rename_all`; the third variant carries an
/// explicit `#[serde(rename = "Image")]` that must win over `rename_all` and keep its case.
fn fixture_enum(rename_all: Option<&str>) -> EnumDef {
    EnumDef {
        name: "SealedThing".to_string(),
        serde_rename_all: rename_all.map(str::to_string),
        variants: vec![
            variant("ElementBased", None),
            variant("Image", None),
            variant("Picture", Some("Image")),
        ],
        ..EnumDef::default()
    }
}

#[test]
fn display_wire_strings_match_the_production_json_name_seam_under_snake_case() {
    let enum_def = fixture_enum(Some("snake_case"));
    let display_text = render_sealed_display("SealedThing", &enum_def, &[], "TestNamespace");

    for v in &enum_def.variants {
        let expected = wire_variant_value(&v.name, v.serde_rename.as_deref(), enum_def.serde_rename_all.as_deref());
        assert!(
            display_text.contains(&format!("=> \"{expected}\"")),
            "variant `{}` should display as \"{expected}\" under snake_case, got:\n{display_text}",
            v.name
        );
    }

    // Regression pin: the bare `Image` variant legitimately lowercases to "image" under
    // snake_case — but the OLD code lowercased the `Picture` variant's explicit
    // `#[serde(rename = "Image")]` too, losing the distinction between the two. Check the
    // `Picture` arm specifically, not just "image" anywhere in the file.
    assert!(
        display_text.contains("SealedThing.Picture => \"Image\","),
        "explicit serde(rename = \"Image\") on `Picture` must keep its case, got:\n{display_text}"
    );
    assert!(
        !display_text.contains("SealedThing.Picture => \"image\","),
        "explicit serde(rename = \"Image\") on `Picture` must not be lowercased, got:\n{display_text}"
    );
}

#[test]
fn display_wire_strings_match_the_production_json_name_seam_under_kebab_case() {
    let enum_def = fixture_enum(Some("kebab-case"));
    let display_text = render_sealed_display("SealedThing", &enum_def, &[], "TestNamespace");

    let expected_element_based = wire_variant_value("ElementBased", None, Some("kebab-case"));
    assert_eq!(expected_element_based, "element-based");
    assert!(
        display_text.contains("=> \"element-based\","),
        "ElementBased should display as \"element-based\" under kebab-case, got:\n{display_text}"
    );
    assert!(
        !display_text.contains("=> \"elementbased\","),
        "kebab-case must not collapse to the bare lowercased variant name, got:\n{display_text}"
    );
}
