//! Coverage for the PHPStan stub half of the declaration-parity fix: `gen_enum_stub` must resolve
//! a variant's reachability identically to the runtime `gen_enum_constants` it describes (see
//! `gen_enum_stub`'s own doc comment on why the two may never independently drift). Split into its
//! own file rather than `type_stubs_tests.rs`, which already sits at its recorded
//! file-modularization ceiling. ~keep
use super::gen_enum_stub;
use crate::backends::php::gen_bindings::types::gen_enum_constants;
use crate::core::ir::{EnumDef, EnumVariant};
use ahash::AHashSet;
use std::collections::HashSet;

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

const CORE_IMPORT: &str = "hostlib";

/// `Base` is always ungated; `Extra` carries `cfg`. `owner` selects whether the whole enum, and
/// therefore `Extra`'s cfg, is host- or foreign-owned relative to [`CORE_IMPORT`] -- the value
/// passed as `core_import` at each call site below.
fn sample_enum(owner: &str, cfg: &str) -> EnumDef {
    EnumDef {
        name: "SampleMode".to_string(),
        rust_path: format!("{owner}::SampleMode"),
        variants: vec![unit_variant("Base", None), unit_variant("Extra", Some(cfg))],
        ..Default::default()
    }
}

/// A foreign-owned `Extra` behind a feature absent from `configured_features` is proven
/// unreachable: the stub must not declare a constant for it, or PHPStan would accept a caller
/// referencing `SampleMode::EXTRA` even though the extension never registers that member.
#[test]
fn foreign_variant_proven_unreachable_is_absent_from_stub_constants() {
    let en = sample_enum("dep_crate", r#"feature = "testkit""#);
    let configured: HashSet<&str> = HashSet::new();

    let stub = gen_enum_stub(&en, &AHashSet::new(), CORE_IMPORT, &configured);

    assert!(stub.contains("public const BASE = 'Base';"), "got:\n{stub}");
    assert!(!stub.contains("EXTRA"), "got:\n{stub}");
}

/// A HOST-owned cfg-gated `Extra` (`rust_path` rooted in the same crate as `core_import`) must
/// never be dropped -- existing behavior, unchanged by this fix -- regardless of
/// `configured_features`.
#[test]
fn host_owned_cfg_gated_variant_stays_on_stub_constants() {
    let en = sample_enum(CORE_IMPORT, r#"feature = "extra_feature""#);
    let configured: HashSet<&str> = HashSet::new();

    let stub = gen_enum_stub(&en, &AHashSet::new(), CORE_IMPORT, &configured);

    assert!(stub.contains("public const BASE = 'Base';"), "got:\n{stub}");
    assert!(stub.contains("public const EXTRA = 'Extra';"), "got:\n{stub}");
}

/// The stub and the runtime `#[php_impl]` block must land on the identical verdict for the same
/// inputs -- cross-checked directly against `gen_enum_constants` (the runtime generator) rather
/// than duplicating expected strings that could independently drift from it.
#[test]
fn stub_and_runtime_agree_on_which_variants_are_declared() {
    let en = sample_enum("dep_crate", r#"feature = "testkit""#);
    let configured: HashSet<&str> = HashSet::new();

    let stub = gen_enum_stub(&en, &AHashSet::new(), CORE_IMPORT, &configured);
    let runtime = gen_enum_constants(&en, None, false, Some(&configured));

    assert!(
        stub.contains("BASE") && runtime.contains("BASE"),
        "stub:\n{stub}\nruntime:\n{runtime}"
    );
    assert!(
        !stub.contains("EXTRA") && !runtime.contains("EXTRA"),
        "stub:\n{stub}\nruntime:\n{runtime}"
    );
}
