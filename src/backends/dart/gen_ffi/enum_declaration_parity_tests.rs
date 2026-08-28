//! Cross-surface parity for a cfg-gated enum variant in `DartStyle::Ffi`: the public Dart `enum`'s
//! case list and the C-FFI crate's `{prefix}_{enum}_from_i32` accepted-discriminant list must
//! always name the same variants.
//!
//! `DartStyle::Ffi` declares `BuildDependency::Ffi` (`gen_bindings::mod.rs::build_config_for`), so
//! `backends::ffi` — not a Dart-owned Rust crate — is this package's paired Rust side. That crate
//! filters the validator's variant list through `codegen::conversions::enum_variant_declaration`
//! (`ffi::gen_bindings::types::declared_variant_indices`), while `gen_ffi::emit` was the one member
//! of the C-FFI consumer family (go, java, kotlin, kotlin_android, csharp, zig all do) that never
//! applied `ApiSurface::with_cfg_filtered_deep` — so the Dart enum advertised a case the linked
//! library's validator rejects.
//!
//! The assertions compare the TWO EMITTED SURFACES to each other, never to a hardcoded literal: a
//! test pinning each side to its own expected string still passes when both sides drift together,
//! which is precisely the defect shape being guarded here. Both surfaces are produced by the real
//! `Backend::generate_bindings` entry points, not by direct calls to the enum emitters.

use super::emit as emit_dart_ffi;
use crate::backends::ffi::FfiBackend;
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

const ENUM_NAME: &str = "RoutingStrategy";
const GATING_FEATURE: &str = "extra-tier";

fn ffi_dart_config(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    // The same feature list on BOTH language tables: `backends::ffi` reads `Language::Ffi`'s set
    // and `gen_ffi` reads `Language::Dart`'s, so leaving one blank would test feature drift
    // (a separate concern `warn_on_ffi_feature_drift` reports) instead of declaration parity. ~keep
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"dart\", \"ffi\"]\n[[crates]]\nname = \"test-lib\"\n\
         sources = [\"src/lib.rs\"]\n[crates.dart]\nstyle = \"ffi\"\n{features_line}\
         [crates.ffi]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A first path segment other than the crate's own name (`test_lib`) is what
/// `is_host_owned_rust_path` reads to classify this enum — and its cfg-gated variant — as FOREIGN.
fn cfg_enum_api(rust_path: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: ENUM_NAME.to_string(),
            rust_path: rust_path.to_string(),
            has_serde: true,
            variants: vec![
                EnumVariant {
                    name: "Primary".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Extra".to_string(),
                    cfg: Some(format!(r#"feature = "{GATING_FEATURE}""#)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Selected by content, not by filename: each backend returns several files, and picking the first
/// by path would silently look at the wrong surface. ~keep
fn file_declaring<'a>(files: &'a [GeneratedFile], suffix: &str, marker: &str) -> &'a str {
    let matches: Vec<&GeneratedFile> = files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with(suffix) && f.content.contains(marker))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one generated {suffix} file containing {marker:?}, found {} ({:?})",
        matches.len(),
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
    );
    &matches[0].content
}

fn body_between<'a>(source: &'a str, header: &str, terminator: &str) -> &'a str {
    let start = source
        .find(header)
        .map(|i| i + header.len())
        .unwrap_or_else(|| panic!("expected {header:?} in:\n{source}"));
    let body = &source[start..];
    let end = body
        .find(terminator)
        .unwrap_or_else(|| panic!("expected {terminator:?} to close {header:?} in:\n{body}"));
    &body[..end]
}

/// Every Rust variant name `{prefix}_routing_strategy_from_i32` accepts a discriminant for.
///
/// `enum_from_i32.jinja` emits one `    {index} => {index}, // {VariantName}` line per accepted
/// variant, so the trailing comment carries the variant's Rust name — the same domain the Dart
/// case list is derived from, which is what lets the two be compared directly. ~keep
fn ffi_accepted_variants(lib_rs: &str) -> Vec<String> {
    let body = body_between(lib_rs, "_routing_strategy_from_i32(value: i32) -> i32 {", "_ =>");
    body.lines()
        .filter_map(|line| line.split_once("// "))
        .map(|(_, name)| name.trim().to_string())
        .collect()
}

/// Every case name the public Dart `enum RoutingStrategy { ... }` declares, in order.
fn dart_cases(dart_src: &str) -> Vec<String> {
    let body = body_between(dart_src, &format!("enum {ENUM_NAME} {{\n"), "\n}");
    body.lines()
        .map(|line| line.trim().trim_end_matches([',', ';']).to_string())
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect()
}

fn assert_surfaces_agree(configured_feature: Option<&str>, rust_path: &str) -> Vec<String> {
    let api = cfg_enum_api(rust_path);
    let config = ffi_dart_config(configured_feature);
    let ffi_files = FfiBackend.generate_bindings(&api, &config).unwrap();
    // `emit` is exactly what `DartBackend::generate_bindings` delegates to for `DartStyle::Ffi`. ~keep
    let dart_files = emit_dart_ffi(&api, &config).unwrap();

    let accepted = ffi_accepted_variants(file_declaring(&ffi_files, ".rs", "_routing_strategy_from_i32"));
    let cases = dart_cases(file_declaring(&dart_files, ".dart", &format!("enum {ENUM_NAME} {{")));
    let expected: Vec<String> = accepted
        .iter()
        .map(|name| public_host_identifier(Language::Dart, PublicIdentifierKind::Field, name))
        .collect();
    assert_eq!(
        cases, expected,
        "the public Dart enum's case list and the C-FFI crate's from_i32 accepted-variant list \
         must name the same variants; FFI accepted {accepted:?}, Dart declared {cases:?}"
    );
    accepted
}

/// The gating feature is NOT configured, so the dependency never compiles `Extra` in: the FFI
/// validator rejects its discriminant, and the Dart enum must not advertise the case.
#[test]
fn dart_case_list_matches_ffi_validator_when_foreign_variant_is_proven_unreachable() {
    let accepted = assert_surfaces_agree(None, "dep_crate::RoutingStrategy");
    assert_eq!(
        accepted,
        vec!["Primary".to_string()],
        "sanity check on the fixture itself: with the gating feature off the FFI validator must \
         already reject the foreign variant, otherwise this test proves nothing"
    );
}

/// Positive control: with the gating feature configured the foreign variant is NOT proven
/// unreachable, so BOTH surfaces must keep it. Without this, dropping every cfg-gated variant from
/// the Dart enum would also satisfy the test above.
#[test]
fn dart_case_list_matches_ffi_validator_when_foreign_variant_is_reachable() {
    let accepted = assert_surfaces_agree(Some(GATING_FEATURE), "dep_crate::RoutingStrategy");
    assert_eq!(
        accepted,
        vec!["Primary".to_string(), "Extra".to_string()],
        "with the gating feature configured the foreign variant is not proven unreachable, so the \
         FFI validator must still accept it"
    );
}
