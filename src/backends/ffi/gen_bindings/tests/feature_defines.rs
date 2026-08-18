//! Pins the per-position coverage of the two independent feature-emitting walks.
//!
//! `codegen::cfg::collect_cfg_features` decides which Cargo features every generated binding
//! crate declares in its `[features]` table, and — via `backends::go::cgo_features` — which `-D`
//! macros the generated cgo preamble passes to the C compiler.
//! `gen_bindings::helpers::cbindgen_feature_defines` decides which `#define`s cbindgen is allowed
//! to guard a header declaration with. They are deliberately NOT unified: the collector applies an
//! `is_host` rust_path filter so it never forwards a feature the core dependency does not define,
//! while the header walk deliberately has none, because a type merged from a foreign
//! `[[crates.source_crates]]` crate still needs its header declaration guarded. Collapsing the two
//! would under-guard the C header.
//!
//! What is checkable is that each walk visits the positions it is supposed to. Both walks are
//! hand-written `for`/`chain` traversals over the same IR, so a new cfg-bearing position added to
//! one is silently missing from the other. The table below is the contract; a walk edited on one
//! side fails here naming the position, not printing two sorted feature lists. ~keep

use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, FieldDef, FunctionDef, MethodDef, ServiceDef, TypeDef,
};
use std::collections::BTreeSet;

/// One cfg-bearing IR position, and whether each walk is expected to reach it.
struct Position {
    /// The IR path, spelled the way the walks index it.
    label: &'static str,
    /// A feature name used at this position and nowhere else, so a set difference is
    /// attributable to exactly one position. ~keep
    feature: &'static str,
    /// Visited by `codegen::cfg::collect_cfg_features`.
    in_collector: bool,
    /// Visited by `backends::ffi::gen_bindings::helpers::cbindgen_feature_defines`.
    in_header_defines: bool,
}

/// The host crate of the fixture surface. `is_host` compares this against each item's leading
/// rust_path segment, so `foreignlib::*` items exercise the filter. ~keep
const HOST_CRATE: &str = "hostlib";

/// Every cfg-bearing position in `ApiSurface`, with the coverage each walk actually has.
///
/// Divergences are intentional-and-documented, or known-and-unfixed; both are called out inline so
/// a future edit that flips a row has to state which it is. ~keep
const POSITIONS: &[Position] = &[
    Position {
        label: "types[].cfg (host)",
        feature: "host-type",
        in_collector: true,
        in_header_defines: true,
    },
    // Field gates reach the collector but not the header walk. Not a header bug: no generator
    // anywhere re-emits a `FieldDef::cfg` as a `#[cfg]` attribute — every consumer of it
    // (`codegen::generators::structs`, `codegen::shared`, the conversion renderers) treats it as a
    // *filter*, and the FFI field accessors at `gen_bindings::lib_rs` are emitted ungated over the
    // already-filtered surface. So no header declaration is ever guarded on a field's feature and
    // no `[defines]` entry could match one. ~keep
    Position {
        label: "types[].fields[].cfg (host)",
        feature: "host-field",
        in_collector: true,
        in_header_defines: false,
    },
    Position {
        label: "types[].methods[].cfg (host)",
        feature: "host-type-method",
        in_collector: true,
        in_header_defines: true,
    },
    Position {
        label: "enums[].cfg (host)",
        feature: "host-enum",
        in_collector: true,
        in_header_defines: true,
    },
    // Variant gates: same reasoning as fields — every reader of `EnumVariant::cfg` filters on it;
    // none re-emits it, so no header declaration is guarded on a variant's feature. ~keep
    Position {
        label: "enums[].variants[].cfg (host)",
        feature: "host-variant",
        in_collector: true,
        in_header_defines: false,
    },
    Position {
        label: "enums[].methods[].cfg (host)",
        feature: "host-enum-method",
        in_collector: true,
        in_header_defines: true,
    },
    Position {
        label: "functions[].cfg (host)",
        feature: "host-function",
        in_collector: true,
        in_header_defines: true,
    },
    Position {
        label: "services[].cfg (host)",
        feature: "host-service",
        in_collector: true,
        in_header_defines: true,
    },
    // The service constructor/configurator gates reach the collector (a Rust-emitting backend
    // re-emits them) but not the header walk, which reads only `ServiceDef::cfg`. The FFI service
    // emitters (`gen_bindings::service_api`) emit no `#[cfg]` for either, so nothing in the header
    // is guarded on them today. A backend change that starts gating a service entry point in the
    // FFI crate must add these two to `cbindgen_feature_defines` or the header declares the symbol
    // unguarded. ~keep
    Position {
        label: "services[].constructor.cfg (host)",
        feature: "host-service-constructor",
        in_collector: true,
        in_header_defines: false,
    },
    Position {
        label: "services[].configurators[].cfg (host)",
        feature: "host-service-configurator",
        in_collector: true,
        in_header_defines: false,
    },
    // The one position the header walk reads and the collector does not. It is currently inert on
    // both sides rather than a gap: `codegen::error_gen::gen_ffi_error_methods` emits the error
    // introspection wrappers with no `#[cfg]` at all, so cbindgen never guards them and the extra
    // `[defines]` entry never matches anything. Whichever side is changed first — teaching
    // `gen_ffi_error_methods` to re-emit `MethodDef::rust_cfg_attribute`, or dropping the read at
    // helpers.rs — the other has to move with it, and this row is what says so. ~keep
    Position {
        label: "errors[].methods[].cfg",
        feature: "error-method",
        in_collector: false,
        in_header_defines: true,
    },
    // The `is_host` axis. Everything below sits on a `foreignlib::` rust_path: the collector must
    // skip it (forwarding `<feat> = ["<core>/<feat>"]` for a feature the core crate does not
    // define breaks cargo resolution) while the header walk must still see it, because the merged
    // item's declaration is real and needs a guard.
    Position {
        label: "types[].cfg (foreign)",
        feature: "foreign-type",
        in_collector: false,
        in_header_defines: true,
    },
    Position {
        label: "types[].fields[].cfg (foreign)",
        feature: "foreign-field",
        in_collector: false,
        in_header_defines: false,
    },
    Position {
        label: "types[].methods[].cfg (foreign)",
        feature: "foreign-type-method",
        in_collector: false,
        in_header_defines: true,
    },
    Position {
        label: "enums[].cfg (foreign)",
        feature: "foreign-enum",
        in_collector: false,
        in_header_defines: true,
    },
    Position {
        label: "enums[].variants[].cfg (foreign)",
        feature: "foreign-variant",
        in_collector: false,
        in_header_defines: false,
    },
    Position {
        label: "enums[].methods[].cfg (foreign)",
        feature: "foreign-enum-method",
        in_collector: false,
        in_header_defines: true,
    },
    // KNOWN DIVERGENCE, deliberately pinned as-is rather than silently fixed: the collector's
    // function loop (src/codegen/cfg.rs:124) has no `is_host` guard, unlike its type, enum and
    // service loops, even though `FunctionDef` carries a `rust_path` and
    // `extract::extractor::reexports::merge_surface` merges foreign functions with the foreign
    // crate's gate. A merged function's feature therefore does leak into the host crate's
    // passthrough table. Flipping this to `false` is the fix; the row exists so the fix is a
    // deliberate edit here rather than an unexplained test failure. ~keep
    Position {
        label: "functions[].cfg (foreign)",
        feature: "foreign-function",
        in_collector: true,
        in_header_defines: true,
    },
    Position {
        label: "services[].cfg (foreign)",
        feature: "foreign-service",
        in_collector: false,
        in_header_defines: true,
    },
    Position {
        label: "services[].constructor.cfg (foreign)",
        feature: "foreign-service-constructor",
        in_collector: false,
        in_header_defines: false,
    },
    Position {
        label: "services[].configurators[].cfg (foreign)",
        feature: "foreign-service-configurator",
        in_collector: false,
        in_header_defines: false,
    },
];

fn gate(feature: &str) -> Option<String> {
    Some(format!("feature = {feature:?}"))
}

fn method(name: &str, feature: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        cfg: gate(feature),
        ..MethodDef::default()
    }
}

fn typ(name: &str, rust_path: &str, type_feature: &str, field_feature: &str, method_feature: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        cfg: gate(type_feature),
        fields: vec![FieldDef {
            name: "gated_field".to_string(),
            cfg: gate(field_feature),
            ..FieldDef::default()
        }],
        methods: vec![method("gated_method", method_feature)],
        ..TypeDef::default()
    }
}

fn enumeration(
    name: &str,
    rust_path: &str,
    enum_feature: &str,
    variant_feature: &str,
    method_feature: &str,
) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        cfg: gate(enum_feature),
        variants: vec![EnumVariant {
            name: "GatedVariant".to_string(),
            cfg: gate(variant_feature),
            ..EnumVariant::default()
        }],
        methods: vec![method("gated_method", method_feature)],
        ..EnumDef::default()
    }
}

fn service(
    name: &str,
    rust_path: &str,
    service_feature: &str,
    constructor_feature: &str,
    configurator_feature: &str,
) -> ServiceDef {
    ServiceDef {
        name: name.to_string(),
        rust_path: rust_path.to_string(),
        constructor: method("new", constructor_feature),
        configurators: vec![method("with_gated_option", configurator_feature)],
        registrations: vec![],
        entrypoints: vec![],
        doc: String::new(),
        cfg: gate(service_feature),
    }
}

/// An API surface carrying a distinct feature name at every cfg-bearing IR position, on both a
/// host-owned and a foreign (`[[crates.source_crates]]`-merged) item of each kind.
fn fixture_api() -> ApiSurface {
    ApiSurface {
        crate_name: HOST_CRATE.to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            typ(
                "HostType",
                "hostlib::HostType",
                "host-type",
                "host-field",
                "host-type-method",
            ),
            typ(
                "ForeignType",
                "foreignlib::ForeignType",
                "foreign-type",
                "foreign-field",
                "foreign-type-method",
            ),
        ],
        enums: vec![
            enumeration(
                "HostEnum",
                "hostlib::HostEnum",
                "host-enum",
                "host-variant",
                "host-enum-method",
            ),
            enumeration(
                "ForeignEnum",
                "foreignlib::ForeignEnum",
                "foreign-enum",
                "foreign-variant",
                "foreign-enum-method",
            ),
        ],
        functions: vec![
            FunctionDef {
                name: "host_function".to_string(),
                rust_path: "hostlib::host_function".to_string(),
                cfg: gate("host-function"),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "foreign_function".to_string(),
                rust_path: "foreignlib::foreign_function".to_string(),
                cfg: gate("foreign-function"),
                ..FunctionDef::default()
            },
        ],
        errors: vec![ErrorDef {
            name: "HostError".to_string(),
            rust_path: "hostlib::HostError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![method("status_code", "error-method")],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        services: vec![
            service(
                "HostService",
                "hostlib::HostService",
                "host-service",
                "host-service-constructor",
                "host-service-configurator",
            ),
            service(
                "ForeignService",
                "foreignlib::ForeignService",
                "foreign-service",
                "foreign-service-constructor",
                "foreign-service-configurator",
            ),
        ],
        ..ApiSurface::default()
    }
}

/// The bare feature names `cbindgen_feature_defines` puts on the key side of cbindgen's
/// `[defines]` table (the keys are `feature = <name>`, unquoted — see the note at that function).
fn header_define_features(api: &ApiSurface) -> BTreeSet<String> {
    super::super::helpers::cbindgen_feature_defines(api, "AL")
        .into_iter()
        .map(|(key, _macro_name)| {
            key.strip_prefix("feature = ")
                .unwrap_or_else(|| panic!("cbindgen [defines] key must be `feature = <name>`, got `{key}`"))
                .to_string()
        })
        .collect()
}

/// Every absence assertion in the coverage test is vacuous unless the fixture really carries a gate
/// at the position the absence is claimed for. This reads the gates straight back out of the IR, so
/// a fixture that quietly stops populating a position fails here first. ~keep
#[test]
fn fixture_carries_a_distinct_gate_at_every_cfg_bearing_position() {
    let api = fixture_api();
    let mut found: Vec<&str> = Vec::new();

    for item in &api.types {
        found.push(item.cfg.as_deref().expect("type gate"));
        found.push(item.fields[0].cfg.as_deref().expect("field gate"));
        found.push(item.methods[0].cfg.as_deref().expect("type method gate"));
    }
    for item in &api.enums {
        found.push(item.cfg.as_deref().expect("enum gate"));
        found.push(item.variants[0].cfg.as_deref().expect("variant gate"));
        found.push(item.methods[0].cfg.as_deref().expect("enum method gate"));
    }
    for item in &api.functions {
        found.push(item.cfg.as_deref().expect("function gate"));
    }
    for item in &api.errors {
        found.push(item.methods[0].cfg.as_deref().expect("error method gate"));
    }
    for item in &api.services {
        found.push(item.cfg.as_deref().expect("service gate"));
        found.push(item.constructor.cfg.as_deref().expect("service constructor gate"));
        found.push(item.configurators[0].cfg.as_deref().expect("service configurator gate"));
    }

    let placed: BTreeSet<String> = found.iter().map(|cfg| cfg.to_string()).collect();
    assert_eq!(
        placed.len(),
        found.len(),
        "each position must use a feature name no other position uses, or a set difference \
         cannot be attributed to one position; got {found:?}"
    );
    for position in POSITIONS {
        let expected = format!("feature = {:?}", position.feature);
        assert!(
            placed.contains(&expected),
            "fixture does not carry a gate at position `{}` (expected `{expected}`); every \
             coverage assertion for that position would be vacuous",
            position.label
        );
    }
    assert_eq!(
        placed.len(),
        POSITIONS.len(),
        "the fixture and the POSITIONS table must describe the same set of positions"
    );
}

/// The invariant: each walk visits exactly the positions the table says it does.
///
/// A future edit that adds a position to `collect_cfg_features` without adding it to
/// `cbindgen_feature_defines` (or the reverse) fails here naming the IR position, so the fix is
/// obvious from the failure message alone. ~keep
#[test]
fn the_two_feature_walks_cover_exactly_the_documented_positions() {
    let api = fixture_api();
    let collector = crate::codegen::cfg::collect_cfg_features(&api);
    let header = header_define_features(&api);

    // Control: assert both walks emitted something before asserting anything about what they did
    // not emit. A walk that returned an empty set would otherwise satisfy every absence row. ~keep
    assert!(
        collector.contains("host-type"),
        "control: collect_cfg_features must reach types[].cfg, got {collector:?}"
    );
    assert!(
        header.contains("host-type"),
        "control: cbindgen_feature_defines must reach types[].cfg, got {header:?}"
    );

    for position in POSITIONS {
        assert_eq!(
            collector.contains(position.feature),
            position.in_collector,
            "codegen::cfg::collect_cfg_features coverage of `{}` changed: the table says \
             visited={}, the walk says visited={}. Update the walk or, if the change is \
             intended, the POSITIONS row — and check whether cbindgen_feature_defines \
             (backends/ffi/gen_bindings/helpers.rs) needs the same edit.",
            position.label,
            position.in_collector,
            collector.contains(position.feature)
        );
        assert_eq!(
            header.contains(position.feature),
            position.in_header_defines,
            "backends::ffi::gen_bindings::helpers::cbindgen_feature_defines coverage of `{}` \
             changed: the table says visited={}, the walk says visited={}. Update the walk or, \
             if the change is intended, the POSITIONS row — and check whether \
             collect_cfg_features (codegen/cfg.rs) needs the same edit.",
            position.label,
            position.in_header_defines,
            header.contains(position.feature)
        );
    }

    let modeled: BTreeSet<&str> = POSITIONS.iter().map(|position| position.feature).collect();
    for feature in collector.iter().chain(header.iter()) {
        assert!(
            modeled.contains(feature.as_str()),
            "a walk emitted `{feature}`, which the POSITIONS table does not model; a new \
             cfg-bearing IR position needs a row here and a decision about the other walk"
        );
    }
}

/// The consumer that makes the two sets' relationship load-bearing rather than cosmetic.
///
/// `backends::go::cgo_features` derives the cgo preamble's `-D` macros from the *collector's*
/// output, while the header's `#if defined(...)` guards come from the *header walk's* output. Any
/// feature the header walk emits that the collector does not is a guard the cgo preamble can never
/// satisfy: cgo then sees no declaration and fails with `could not determine what C.<symbol>
/// refers to`. Today the only such feature is the inert `errors[].methods[].cfg` read, plus the
/// `is_host`-filtered foreign items — whose gates are also absent from the FFI crate's
/// `[features]`, so the export is compiled out of the cdylib and the guard being false is correct.
/// This test states which features are in that gap so a new entry has to be justified. ~keep
#[test]
fn the_header_only_features_are_the_documented_ones() {
    let api = fixture_api();
    let collector = crate::codegen::cfg::collect_cfg_features(&api);
    let header = header_define_features(&api);

    let header_only: BTreeSet<&str> = header
        .iter()
        .map(String::as_str)
        .filter(|feature| !collector.contains(*feature))
        .collect();

    let expected: BTreeSet<&str> = POSITIONS
        .iter()
        .filter(|position| position.in_header_defines && !position.in_collector)
        .map(|position| position.feature)
        .collect();

    assert!(
        !expected.is_empty(),
        "control: the table must model at least one header-only position, or this test asserts \
         nothing"
    );
    assert_eq!(
        header_only, expected,
        "the set of features guarding the C header but absent from every binding crate's \
         [features] table (and so from backends::go::cgo_features' -D list) changed"
    );
}
