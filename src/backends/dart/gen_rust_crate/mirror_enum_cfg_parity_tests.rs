//! Table-driven regression coverage for the mirror-declaration/conversion cfg parity fix
//! (`ConversionConfig::declaration_drops_unreachable_foreign_variants`, commit f9795aea9).
//!
//! `mirror::emit_mirror_enum` (the Dart bridge crate's own enum declaration) keeps a FOREIGN
//! cfg-gated variant unconditionally -- it never emits a per-variant `#[cfg(...)]` attribute at
//! all, so the variant is always compiled into the mirror regardless of `configured_features`.
//! Both `enum_conversions::emit_from_mirror_to_core_enum` (mirror -> core) and
//! `enum_conversions::emit_from_impl_for_enum` (core -> mirror) drop the match ARM for that same
//! variant unconditionally (`emit_cfg_gated_arm`'s rule: a wrapper crate cannot forward a foreign
//! crate's feature as its own gate). Whether dropping the arm also leaves the match
//! non-exhaustive depends on whether the type actually being matched can still hold the variant:
//!
//! - mirror -> core matches the Mirror enum this crate declares, which -- per the paragraph
//!   above -- ALWAYS still holds the variant. The catch-all is therefore required whenever a
//!   foreign cfg-gated variant exists, independent of `configured_features`.
//! - core -> mirror matches the real core type, a shape this crate does not declare. Once
//!   `configured_features` proves the dependency itself never compiles the variant in, the
//!   match is already exhaustive without an arm for it, and a catch-all would be dead code
//!   (`unreachable_patterns`).
//!
//! A single fixture proves only that one enum shape got the right answer. The two defects this
//! guards against are shape-independent by construction (`continue`s before any variant-shape
//! branch runs, see both functions under test), so what actually needs multi-instance coverage is
//! the *combination space* the resolver's boolean inputs create: single vs. multiple foreign
//! cfg-gated variants, proven vs. unproven vs. unknown reachability, host- vs. foreign-owned
//! enums, and the orthogonal `excluded_variants` gap that only the core -> mirror direction can
//! have. Each row below is a DISTINCT `EnumDef` (distinct name, distinct shape) exercising one
//! point in that space, and each row asserts on BOTH the mirror declaration output and both
//! conversion directions together -- so a row fails loudly if declaration and conversion ever
//! drift apart again for that shape, which is exactly the defect the consumer's audit named. ~keep

use super::enum_conversions::{emit_from_impl_for_enum, emit_from_mirror_to_core_enum};
use super::mirror::emit_mirror_enum;
use crate::core::ir::{EnumDef, EnumVariant};

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

/// One row: a distinct enum shape, the binding's configured features, and the expected verdict
/// for the mirror declaration and both conversion directions.
struct Case {
    description: &'static str,
    enum_def: EnumDef,
    /// `None` means "unknown" (the conservative default); `Some(vec![])` means the binding
    /// configures no features at all, which proves any `feature = "x"` gate unsatisfied. ~keep
    configured_features: Option<Vec<String>>,
    /// Every foreign cfg-gated variant name that the mirror declaration must still contain. ~keep
    mirror_keeps_variants: &'static [&'static str],
    mirror_to_core_needs_catch_all: bool,
    core_to_mirror_needs_catch_all: bool,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            description: "single foreign cfg variant, feature set unknown (None)",
            enum_def: EnumDef {
                name: "RoutingMode".to_string(),
                rust_path: "dep_crate::RoutingMode".to_string(),
                variants: vec![
                    unit_variant("Direct", None),
                    unit_variant("Relayed", Some(r#"feature = "relay""#)),
                ],
                ..Default::default()
            },
            configured_features: None,
            mirror_keeps_variants: &["Relayed"],
            mirror_to_core_needs_catch_all: true,
            core_to_mirror_needs_catch_all: true,
        },
        Case {
            description: "single foreign cfg variant, proven unreachable (feature not configured)",
            enum_def: EnumDef {
                name: "CompressionKind".to_string(),
                rust_path: "dep_crate::CompressionKind".to_string(),
                variants: vec![
                    unit_variant("None", None),
                    unit_variant("Brotli", Some(r#"feature = "brotli""#)),
                ],
                ..Default::default()
            },
            configured_features: Some(vec![]),
            mirror_keeps_variants: &["Brotli"],
            // Mirror declaration never drops it, so mirror -> core still needs the catch-all even
            // though the core -> mirror direction (matched against the real, feature-proven core
            // type) does not. ~keep
            mirror_to_core_needs_catch_all: true,
            core_to_mirror_needs_catch_all: false,
        },
        Case {
            description: "single foreign cfg variant, feature configured (not proven unreachable)",
            enum_def: EnumDef {
                name: "RetryPolicy".to_string(),
                rust_path: "dep_crate::RetryPolicy".to_string(),
                variants: vec![
                    unit_variant("Fixed", None),
                    unit_variant("Backoff", Some(r#"feature = "backoff""#)),
                ],
                ..Default::default()
            },
            configured_features: Some(vec!["backoff".to_string()]),
            mirror_keeps_variants: &["Backoff"],
            mirror_to_core_needs_catch_all: true,
            core_to_mirror_needs_catch_all: true,
        },
        Case {
            description: "multiple foreign cfg variants, mixed reachability (OR semantics)",
            enum_def: EnumDef {
                name: "TransportKind".to_string(),
                rust_path: "dep_crate::TransportKind".to_string(),
                variants: vec![
                    unit_variant("Tcp", None),
                    // Proven unreachable on its own -- "quic" is not in configured_features below. ~keep
                    unit_variant("Quic", Some(r#"feature = "quic""#)),
                    // NOT proven unreachable -- "websocket" is configured below. ~keep
                    unit_variant("WebSocket", Some(r#"feature = "websocket""#)),
                ],
                ..Default::default()
            },
            configured_features: Some(vec!["websocket".to_string()]),
            mirror_keeps_variants: &["Quic", "WebSocket"],
            mirror_to_core_needs_catch_all: true,
            // At least one unresolved foreign variant (WebSocket) is enough to require the
            // catch-all even though Quic alone would already have been proven unreachable. ~keep
            core_to_mirror_needs_catch_all: true,
        },
        Case {
            description: "multiple foreign cfg variants, ALL proven unreachable",
            enum_def: EnumDef {
                name: "AccelerationBackend".to_string(),
                rust_path: "dep_crate::AccelerationBackend".to_string(),
                variants: vec![
                    unit_variant("Cpu", None),
                    unit_variant("Cuda", Some(r#"feature = "cuda""#)),
                    unit_variant("Rocm", Some(r#"feature = "rocm""#)),
                ],
                ..Default::default()
            },
            configured_features: Some(vec![]),
            mirror_keeps_variants: &["Cuda", "Rocm"],
            mirror_to_core_needs_catch_all: true,
            core_to_mirror_needs_catch_all: false,
        },
        Case {
            description: "host-owned enum cfg variant never needs a catch-all in either direction",
            enum_def: EnumDef {
                name: "LogLevel".to_string(),
                // "mylib" matches the source_crate_name every case below is emitted with, so
                // `is_host_owned_rust_path` classifies this whole enum (and thus every variant)
                // as host-owned -- the crate declares this feature itself. ~keep
                rust_path: "mylib::LogLevel".to_string(),
                variants: vec![
                    unit_variant("Info", None),
                    unit_variant("Trace", Some(r#"feature = "verbose-logging""#)),
                ],
                ..Default::default()
            },
            configured_features: Some(vec![]),
            mirror_keeps_variants: &["Trace"],
            mirror_to_core_needs_catch_all: false,
            core_to_mirror_needs_catch_all: false,
        },
        Case {
            description: "no cfg variants at all -- negative control, neither direction needs a catch-all",
            enum_def: EnumDef {
                name: "SortOrder".to_string(),
                rust_path: "dep_crate::SortOrder".to_string(),
                variants: vec![unit_variant("Ascending", None), unit_variant("Descending", None)],
                ..Default::default()
            },
            configured_features: Some(vec![]),
            mirror_keeps_variants: &[],
            mirror_to_core_needs_catch_all: false,
            core_to_mirror_needs_catch_all: false,
        },
    ]
}

/// The regression this task guards against, across every shape in [`cases`] at once: for each
/// row, (1) the mirror declaration must still contain every foreign cfg-gated variant verbatim
/// (matching what `mirror::emit_mirror_enum` actually does -- it has no per-variant cfg template
/// at all), and (2) the mirror -> core and core -> mirror catch-all decisions must match the
/// row's expectation exactly. Asserting both together is what catches drift: a resolver that
/// silently reverted to a single shared verdict for both directions (the pre-fix bug) would still
/// pass a test that only checked declaration-keeps-variant, or only checked one direction's
/// catch-all, but fails this one at the first row where the two directions' expectations differ
/// (every row except the negative controls). ~keep
#[test]
fn mirror_declaration_and_conversion_catch_all_agree_across_enum_shapes() {
    for case in cases() {
        let mut mirror_out = String::new();
        emit_mirror_enum(&mut mirror_out, &case.enum_def);
        for variant_name in case.mirror_keeps_variants {
            assert!(
                mirror_out.contains(*variant_name),
                "[{}] mirror declaration for {} must retain foreign cfg-gated variant {} \
                 unconditionally, got:\n{mirror_out}",
                case.description,
                case.enum_def.name,
                variant_name,
            );
        }

        let mut mirror_to_core = String::new();
        emit_from_mirror_to_core_enum(
            &mut mirror_to_core,
            &case.enum_def,
            "mylib",
            case.configured_features.as_deref(),
        );
        assert_eq!(
            mirror_to_core.contains("_ => unreachable!"),
            case.mirror_to_core_needs_catch_all,
            "[{}] mirror->core catch-all presence for {} was {}, expected {}. Generated:\n{mirror_to_core}",
            case.description,
            case.enum_def.name,
            mirror_to_core.contains("_ => unreachable!"),
            case.mirror_to_core_needs_catch_all,
        );

        let mut core_to_mirror = String::new();
        emit_from_impl_for_enum(
            &mut core_to_mirror,
            &case.enum_def,
            "mylib",
            case.configured_features.as_deref(),
        );
        assert_eq!(
            core_to_mirror.contains("_ => unreachable!"),
            case.core_to_mirror_needs_catch_all,
            "[{}] core->mirror catch-all presence for {} was {}, expected {}. Generated:\n{core_to_mirror}",
            case.description,
            case.enum_def.name,
            core_to_mirror.contains("_ => unreachable!"),
            case.core_to_mirror_needs_catch_all,
        );
    }
}

/// Orthogonal to cfg entirely: `excluded_variants` is a core -> mirror-only gap (a core variant
/// this binding never generates an arm for, regardless of any cfg). Mirror -> core can never have
/// this gap by construction -- it only ever matches the mirror's OWN declared variants -- so this
/// row proves the two directions are not accidentally sharing one verdict for this input either. ~keep
#[test]
fn excluded_variants_only_forces_a_catch_all_on_the_core_to_mirror_direction() {
    let en = EnumDef {
        name: "PayloadKind".to_string(),
        rust_path: "dep_crate::PayloadKind".to_string(),
        variants: vec![unit_variant("Text", None), unit_variant("Binary", None)],
        excluded_variants: vec![unit_variant("StreamHandle", None)],
        ..Default::default()
    };

    let mut mirror_to_core = String::new();
    emit_from_mirror_to_core_enum(&mut mirror_to_core, &en, "mylib", Some(&[]));
    assert!(
        !mirror_to_core.contains("_ => unreachable!"),
        "mirror->core has no excluded-variants gap and no cfg-gated variant, got:\n{mirror_to_core}"
    );

    let mut core_to_mirror = String::new();
    emit_from_impl_for_enum(&mut core_to_mirror, &en, "mylib", Some(&[]));
    assert!(
        core_to_mirror.contains("_ => unreachable!"),
        "core->mirror must cover the excluded StreamHandle variant with a catch-all, got:\n{core_to_mirror}"
    );
}
