//! Table-driven regression coverage for mirror-declaration/conversion cfg parity.
//!
//! `mirror::emit_mirror_enum` (the Dart bridge crate's own enum declaration) now asks the same
//! `codegen::conversions::enums::enum_variant_declaration` authority every conversion arm below
//! (and every other backend's own wrapper declaration) already consults: a FOREIGN cfg-gated
//! variant is declared only when this binding's own `configured_features` does NOT prove it
//! unreachable. Before this fix the mirror declared every variant unconditionally regardless of
//! `configured_features` -- a Dart caller could construct a value the real dependency build never
//! compiles in, and passing it back into Rust hit the `mirror -> core` catch-all's
//! `unreachable!()` at runtime instead of failing to compile at all. That mismatch (declaration
//! says "exists", conversion says "impossible") is exactly the round-trip failure the consumer's
//! audit reported for Dart specifically, while every other Rust-emitting backend already agreed
//! declaration and conversion via the same authority.
//!
//! Both `enum_conversions::emit_from_mirror_to_core_enum` (mirror -> core) and
//! `enum_conversions::emit_from_impl_for_enum` (core -> mirror) drop the match ARM for a foreign
//! cfg-gated variant unconditionally (`emit_cfg_gated_arm`'s rule: a wrapper crate cannot forward
//! a foreign crate's feature as its own gate). Whether dropping the arm also leaves the match
//! non-exhaustive depends on whether the type actually being matched can still hold the variant:
//!
//! - mirror -> core matches the Mirror enum this crate declares, which -- per the paragraph
//!   above -- now holds the variant ONLY when `configured_features` does not prove it
//!   unreachable. The catch-all is therefore required in exactly that same case, never
//!   unconditionally.
//! - core -> mirror matches the real core type, a shape this crate does not declare. Once
//!   `configured_features` proves the dependency itself never compiles the variant in, the
//!   match is already exhaustive without an arm for it, and a catch-all would be dead code
//!   (`unreachable_patterns`).
//!
//! Both directions now resolve `declaration_may_drop_variant = true`, so the two decisions can
//! never drift out of the SAME shape again.
//!
//! A single fixture proves only that one enum shape got the right answer. The two defects this
//! guards against are shape-independent by construction (`continue`s before any variant-shape
//! branch runs, see both functions under test), so what actually needs multi-instance coverage is
//! the *combination space* the resolver's boolean inputs create: single vs. multiple foreign
//! cfg-gated variants, proven vs. unproven vs. unknown reachability, host- vs. foreign-owned
//! enums, and the orthogonal `excluded_variants` gap that only the core -> mirror direction can
//! have. Each row below is a DISTINCT `EnumDef` (distinct name, distinct shape) exercising one
//! point in that space, and each row asserts on BOTH the mirror declaration output (presence AND
//! absence, on the exact parsed variant-name list -- not a substring `.contains` check, which
//! would pass even if the wrong variant were dropped) and both conversion directions together --
//! so a row fails loudly if declaration and conversion ever drift apart again for that shape. ~keep

use super::enum_conversions::{emit_from_impl_for_enum, emit_from_mirror_to_core_enum};
use super::mirror::emit_mirror_enum;
use crate::core::ir::{EnumDef, EnumVariant};

/// Parse every `pub enum {name} { ... }` unit-variant declaration line out of `mirror_out`
/// (each line renders as `    VariantName,` per `rust_mirror_enum_unit_variant.jinja`) into an
/// exact, ordered set of declared variant names -- an exact membership check on a parsed list,
/// not a substring `.contains` scan that a longer identifier could satisfy by accident.
fn declared_unit_variant_names(mirror_out: &str) -> Vec<&str> {
    mirror_out
        .lines()
        .filter_map(|line| line.trim().strip_suffix(','))
        .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect()
}

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
    /// The EXACT, ordered set of variant names the mirror declaration must contain -- an exact
    /// list, not just "these names must appear somewhere", so a row also proves a dropped
    /// foreign variant is genuinely ABSENT rather than merely not asserted on. ~keep
    mirror_declared_variants: &'static [&'static str],
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
            mirror_declared_variants: &["Direct", "Relayed"],
            mirror_to_core_needs_catch_all: true,
            core_to_mirror_needs_catch_all: true,
        },
        Case {
            description: "single foreign cfg variant, proven unreachable (feature not configured)",
            enum_def: EnumDef {
                name: "CompressionKind".to_string(),
                rust_path: "dep_crate::CompressionKind".to_string(),
                variants: vec![
                    unit_variant("Uncompressed", None),
                    unit_variant("Brotli", Some(r#"feature = "brotli""#)),
                ],
                ..Default::default()
            },
            configured_features: Some(vec![]),
            // Brotli is now DROPPED from the declaration: `configured_features` proves it
            // unreachable, and the mirror asks the same authority the conversions already do. ~keep
            mirror_declared_variants: &["Uncompressed"],
            // Declaration and mirror->core now agree the variant cannot exist, so neither
            // direction needs a catch-all for it any more. ~keep
            mirror_to_core_needs_catch_all: false,
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
            mirror_declared_variants: &["Fixed", "Backoff"],
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
            // Quic is dropped (proven unreachable); WebSocket is kept (not proven). ~keep
            mirror_declared_variants: &["Tcp", "WebSocket"],
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
            // Both foreign variants are proven unreachable and dropped entirely. ~keep
            mirror_declared_variants: &["Cpu"],
            mirror_to_core_needs_catch_all: false,
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
            // Host-owned variants are always kept regardless of `configured_features` --
            // `enum_variant_declaration` never resolves a host-owned gate to `Drop`. ~keep
            mirror_declared_variants: &["Info", "Trace"],
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
            mirror_declared_variants: &["Ascending", "Descending"],
            mirror_to_core_needs_catch_all: false,
            core_to_mirror_needs_catch_all: false,
        },
    ]
}

/// The regression this task guards against, across every shape in [`cases`] at once: for each
/// row, (1) the mirror declaration must still contain every foreign cfg-gated variant verbatim
/// (an EXACT parsed list, so a dropped variant's absence is verified, not merely a kept
/// variant's presence), and (2) the mirror -> core and core -> mirror catch-all decisions must
/// match the row's expectation exactly. Asserting both together is what catches drift: a
/// resolver that silently reverted to a single shared verdict for both directions (the pre-fix
/// bug) would still pass a test that only checked declaration-keeps-variant, or only checked one
/// direction's catch-all, but fails this one at the first row where the two directions'
/// expectations differ (every row except the negative controls). ~keep
#[test]
fn mirror_declaration_and_conversion_catch_all_agree_across_enum_shapes() {
    for case in cases() {
        let mut mirror_out = String::new();
        emit_mirror_enum(
            &mut mirror_out,
            &case.enum_def,
            "mylib",
            case.configured_features.as_deref(),
        );
        assert_eq!(
            declared_unit_variant_names(&mirror_out),
            case.mirror_declared_variants,
            "[{}] mirror declaration for {} must declare exactly {:?}, got:\n{mirror_out}",
            case.description,
            case.enum_def.name,
            case.mirror_declared_variants,
        );

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

fn unit_variant_with_doc(name: &str, cfg: Option<&str>, doc: &str) -> EnumVariant {
    EnumVariant {
        doc: doc.to_string(),
        ..unit_variant(name, cfg)
    }
}

/// Exact reproduction of the consumer's reported shape: a foreign enum with two always-present
/// variants and one variant gated behind a feature ("testkit") this binding does not configure
/// in production. The excluded variant must vanish from the declaration, from the doc comment
/// text, AND from anywhere its name could leak -- not merely be un-asserted. The second half is
/// the control: the identical enum with "testkit" actually configured must retain it, proving
/// the drop is conditional on `configured_features` rather than a blanket "foreign means gone". ~keep
#[test]
fn foreign_cfg_excluded_variant_vanishes_from_mirror_declaration_and_docs_unless_active() {
    fn sync_mode_enum() -> EnumDef {
        EnumDef {
            name: "SyncMode".to_string(),
            rust_path: "dep_crate::SyncMode".to_string(),
            variants: vec![
                unit_variant_with_doc("Manual", None, "Synced only when explicitly requested."),
                unit_variant_with_doc("Automatic", None, "Synced on every change."),
                unit_variant_with_doc(
                    "Testkit",
                    Some(r#"feature = "testkit""#),
                    "Only available in test builds.",
                ),
            ],
            ..Default::default()
        }
    }

    // Production shape: "testkit" is not configured for this binding, so it is proven
    // unreachable and must be dropped everywhere.
    let en = sync_mode_enum();
    let mut out = String::new();
    emit_mirror_enum(&mut out, &en, "mylib", Some(&[]));
    assert_eq!(
        declared_unit_variant_names(&out),
        ["Manual", "Automatic"],
        "excluded-variant mirror must declare exactly the two retained variants, got:\n{out}"
    );
    assert!(
        !out.contains("Testkit"),
        "the excluded variant's name must not appear anywhere in the mirror declaration, got:\n{out}"
    );
    assert!(
        !out.contains("Only available in test builds."),
        "the excluded variant's doc comment must not survive into the mirror declaration, got:\n{out}"
    );

    let mut mirror_to_core = String::new();
    emit_from_mirror_to_core_enum(&mut mirror_to_core, &en, "mylib", Some(&[]));
    assert!(
        !mirror_to_core.contains("Testkit"),
        "the serialization mapping (mirror -> core) must not reference the excluded variant, got:\n{mirror_to_core}"
    );
    let mut core_to_mirror = String::new();
    emit_from_impl_for_enum(&mut core_to_mirror, &en, "mylib", Some(&[]));
    assert!(
        !core_to_mirror.contains("Testkit"),
        "the serialization mapping (core -> mirror) must not reference the excluded variant, got:\n{core_to_mirror}"
    );

    // Control: the SAME enum shape, but with "testkit" actually configured -- the variant is no
    // longer proven unreachable, so it must be retained everywhere, proving the drop above was
    // conditional on `configured_features` rather than foreign ownership alone.
    let en_active = sync_mode_enum();
    let active_features = vec!["testkit".to_string()];
    let mut out_active = String::new();
    emit_mirror_enum(&mut out_active, &en_active, "mylib", Some(&active_features));
    assert_eq!(
        declared_unit_variant_names(&out_active),
        ["Manual", "Automatic", "Testkit"],
        "with \"testkit\" configured, the mirror must retain all three variants, got:\n{out_active}"
    );
    assert!(
        out_active.contains("Only available in test builds."),
        "with \"testkit\" configured, the variant's doc comment must be present, got:\n{out_active}"
    );
}
