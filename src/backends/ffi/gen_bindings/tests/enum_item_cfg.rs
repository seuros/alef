// Separate file rather than `cfg_gated_variants.rs`: that file covers a *variant's* own cfg
// inside the `from_i32` reconstruction helper. This file covers the *enum item's* own cfg --
// whether it propagates into the handle-accessor functions (`_free`, `_to_json`, `_to_string`,
// `_from_json`) and the private `from_i32_rs` helper, the same way `typ.cfg` already propagates
// into the struct-side equivalents (`gen_type_free`, `gen_type_new`, ...). ~keep
use super::super::types::{
    gen_enum_free, gen_enum_from_i32_rs_helper, gen_enum_from_json, gen_enum_to_json, gen_enum_to_string,
};
use crate::core::ir::*;

const HOST_CRATE: &str = "sample_lib";
const PREFIX: &str = "sample_lib";

/// An enum defined inside a `#[cfg(feature = "thumbnails")]`-gated module: the core type itself
/// (`sample_lib::thumbnails::ThumbnailKind`) does not exist in a build without that feature.
fn gated_enum() -> EnumDef {
    EnumDef {
        name: "ThumbnailKind".to_string(),
        rust_path: format!("{HOST_CRATE}::thumbnails::ThumbnailKind"),
        cfg: Some("feature = \"thumbnails\"".to_string()),
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Small".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Large".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// An otherwise identical enum with no item-level cfg, for the negative control: an ungated
/// enum's generated functions must not grow a `#[cfg(...)]` line, and must stay byte-for-byte
/// unchanged apart from the enum's own name/path.
fn ungated_enum() -> EnumDef {
    EnumDef {
        name: "PdfRenderMode".to_string(),
        rust_path: format!("{HOST_CRATE}::PdfRenderMode"),
        cfg: None,
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Small".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Large".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

#[test]
fn enum_free_carries_the_enum_items_own_cfg() {
    let rendered = gen_enum_free(&gated_enum(), PREFIX, HOST_CRATE);
    assert!(
        rendered.contains("#[cfg(feature = \"thumbnails\")]"),
        "a handle accessor for a cfg-gated enum must carry that enum's #[cfg], got:\n{rendered}"
    );
    // The gate must precede the item it guards, but doc comments and other attributes may sit
    // between them -- all of them attach to the same item, so pinning an exact line offset would
    // fail on a purely cosmetic reordering. What must hold is: exactly one gate, and it comes
    // before the function it guards. ~keep
    assert_eq!(
        rendered.matches("#[cfg(").count(),
        1,
        "exactly one gate, got:\n{rendered}"
    );
    let cfg_line = rendered.lines().position(|l| l.contains("#[cfg(")).expect("cfg line");
    let fn_line = rendered
        .lines()
        .position(|l| l.contains("pub unsafe extern \"C\" fn"))
        .expect("fn line");
    assert!(cfg_line < fn_line, "#[cfg] must precede the function, got:\n{rendered}");
}

#[test]
fn enum_to_json_carries_the_enum_items_own_cfg() {
    let rendered = gen_enum_to_json(&gated_enum(), PREFIX, HOST_CRATE);
    assert!(
        rendered.contains("#[cfg(feature = \"thumbnails\")]"),
        "got:\n{rendered}"
    );
}

#[test]
fn enum_to_string_carries_the_enum_items_own_cfg() {
    let rendered = gen_enum_to_string(&gated_enum(), PREFIX, HOST_CRATE);
    assert!(
        rendered.contains("#[cfg(feature = \"thumbnails\")]"),
        "got:\n{rendered}"
    );
}

#[test]
fn enum_from_json_carries_the_enum_items_own_cfg() {
    let rendered = gen_enum_from_json(&gated_enum(), PREFIX, HOST_CRATE);
    assert!(
        rendered.contains("#[cfg(feature = \"thumbnails\")]"),
        "got:\n{rendered}"
    );
}

/// The exact shape observed in the field: a private reconstruction helper referencing the gated
/// core type in its own signature (`Option<sample_lib::thumbnails::ThumbnailKind>`), emitted with
/// no gate at all before this fix -- a hard `E0433` in any build where `thumbnails` is declared
/// (e.g. via `[crates.ffi].extra_features`) but not enabled by default.
#[test]
fn from_i32_rs_helper_carries_the_enum_items_own_cfg() {
    let rendered = gen_enum_from_i32_rs_helper(&gated_enum(), HOST_CRATE, HOST_CRATE);
    assert!(
        rendered.contains("#[cfg(feature = \"thumbnails\")]"),
        "the from_i32_rs helper for a cfg-gated enum must carry that enum's #[cfg], got:\n{rendered}"
    );
    let cfg_line = rendered.lines().position(|l| l.contains("#[cfg(")).expect("cfg line");
    let fn_line = rendered.lines().position(|l| l.contains("fn ")).expect("fn line");
    assert!(
        cfg_line < fn_line,
        "#[cfg] must precede the fn declaration, got:\n{rendered}"
    );
}

/// Negative control: an enum with no item-level cfg must not gain one. Every one of the four
/// handle-accessor generators is exercised so a fix that only threads `source_cfg` through one of
/// them (leaving the others silently unguarded) cannot pass this file.
#[test]
fn ungated_enum_emits_no_cfg_attribute_anywhere() {
    let enum_def = ungated_enum();
    for rendered in [
        gen_enum_free(&enum_def, PREFIX, HOST_CRATE),
        gen_enum_to_json(&enum_def, PREFIX, HOST_CRATE),
        gen_enum_to_string(&enum_def, PREFIX, HOST_CRATE),
        gen_enum_from_json(&enum_def, PREFIX, HOST_CRATE),
        gen_enum_from_i32_rs_helper(&enum_def, HOST_CRATE, HOST_CRATE),
    ] {
        assert!(
            !rendered.contains("#[cfg("),
            "an enum with no item-level cfg must not emit a #[cfg] attribute, got:\n{rendered}"
        );
    }
}
