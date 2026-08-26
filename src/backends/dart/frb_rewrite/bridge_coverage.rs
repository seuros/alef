//! Verify that flutter_rust_bridge's generated Dart bridge exposes every free function
//! declared in alef's generated FRB facade (`packages/dart/rust/src/lib.rs`).
//!
//! `flutter_rust_bridge_codegen` is invoked via [`crate::core::backend::PostBuildStep::RunCommand`],
//! and that step's runner treats a missing tool (or the `ALEF_SKIP_COMMANDS` escape hatch) as a
//! non-fatal skip -- deliberately, so a host without `flutter_rust_bridge_codegen` installed can
//! still regenerate the Rust-side facade and fall back to whatever bridge Dart source is already
//! committed. The [`PostProcessFile`](crate::core::backend::PostBuildStep::PostProcessFile) steps
//! that follow it in the post-build sequence (sealed-variant rewriting, extension injection, ...)
//! run unconditionally, patching whatever is on disk regardless of whether frb actually produced
//! it this run.
//!
//! That combination is silently unsound whenever the facade changed since the bridge was last
//! regenerated: alef's own patches land on a stale bridge, producing a file that looks freshly
//! post-processed but is missing every function the facade gained in the meantime (alef #135).
//! [`missing_bridge_functions`] is the invariant that [`crate::cli::pipeline::commands::build::frb_bridge_coverage`]
//! checks immediately after the `RunCommand` step and before any `PostProcessFile` rewrite, so a
//! stale bridge fails the build loudly instead of being silently patched into a half-correct state.
//!
//! A facade function can also be legitimately absent from the bridge with frb having done nothing
//! wrong at all: flutter_rust_bridge's codegen macro expansion runs against the crate's own
//! `default` Cargo features (see the `cfg_gates` module doc), so a `#[cfg(feature = "X")]`-gated
//! function whose feature is not in that manifest's `default` list is correctly invisible to frb
//! and must not be reported missing. [`missing_bridge_functions`] takes the enabled-feature set
//! callers resolve from that same manifest (via
//! [`crate::codegen::cfg::read_default_enabled_cargo_features`]) and skips any facade function
//! whose gate that set does not satisfy.

use super::cfg_gates::{cfg_gated_free_functions, free_pub_fn_name};
use super::text_transformations::{contains_function_at_token_boundary, snake_to_camel};
use std::collections::{HashMap, HashSet};

/// Names of every top-level (column 0) `pub fn` / `pub async fn` free function declared in
/// `lib_rs_source`, in declaration order.
///
/// This is the full set flutter_rust_bridge should bridge from the facade -- unlike
/// [`super::cfg_gates::cfg_gated_free_functions`], which only returns the `#[cfg(...)]`-gated
/// subset.
pub fn free_function_names(lib_rs_source: &str) -> Vec<String> {
    lib_rs_source.lines().filter_map(free_pub_fn_name).collect()
}

/// Strip a `#[cfg(...)]` attribute's `#[cfg(` / `)]` delimiters, returning the bare predicate
/// text [`crate::core::ir::cfg_feature_satisfied`] expects (e.g. `feature = "premium-tier"`).
///
/// `gate` is always produced by [`super::cfg_gates::cfg_gated_free_functions`], which guarantees
/// the `#[cfg(` prefix and `)]` suffix are present verbatim.
fn cfg_predicate(gate: &str) -> &str {
    gate.strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
        .unwrap_or(gate)
}

/// Names of every top-level free function in `lib_rs_source` that flutter_rust_bridge would
/// actually see and bridge, given `enabled_features`.
///
/// A function with no `#[cfg(...)]` gate is always included. A gated function is included only
/// when `enabled_features` satisfies its gate (evaluated with the same
/// [`crate::core::ir::cfg_feature_satisfied`] every other cfg filter in this repo uses). When
/// `enabled_features` is `None` -- the manifest that would supply it could not be read or parsed
/// -- every function, gated or not, is included: this is the pre-cfg-awareness behavior, so an
/// unreadable manifest degrades to the old blanket check rather than silently exempting every
/// gated function from coverage.
fn active_free_function_names(lib_rs_source: &str, enabled_features: Option<&HashSet<&str>>) -> Vec<String> {
    let gates: HashMap<String, String> = cfg_gated_free_functions(lib_rs_source).into_iter().collect();
    free_function_names(lib_rs_source)
        .into_iter()
        .filter(|name| {
            let Some(gate) = gates.get(name) else {
                return true;
            };
            let Some(features) = enabled_features else {
                return true;
            };
            crate::core::ir::cfg_feature_satisfied(Some(cfg_predicate(gate)), features)
        })
        .collect()
}

/// Names (as declared in `lib_rs_source`, snake_case) of facade free functions that have no
/// matching function in `bridge_dart_source`.
///
/// `exclude_functions` are removed from consideration: those are deliberately stripped from the
/// bridge post-frb by [`super::text_transformations::filter_excluded_functions`] and are expected
/// to be permanently absent, independent of whether the bridge is otherwise fresh.
///
/// `enabled_features` -- see [`active_free_function_names`] -- excludes a `#[cfg(...)]`-gated
/// function from consideration when its gate is not satisfied, since frb never saw it either.
///
/// Matching uses flutter_rust_bridge's snake_case -> lowerCamelCase convention and a
/// token-boundary lookup ([`contains_function_at_token_boundary`]) rather than a literal
/// leading-space check, so a function whose name `dartfmt` wrapped onto its own line (a long
/// return type pushes it past the line's whitespace) is still found (alef #191). This is the
/// same boundary-safe lookup [`super::text_transformations::filter_excluded_functions`] uses to
/// find a function to strip.
pub fn missing_bridge_functions(
    lib_rs_source: &str,
    bridge_dart_source: &str,
    exclude_functions: &[String],
    enabled_features: Option<&HashSet<&str>>,
) -> Vec<String> {
    let excluded: HashSet<&str> = exclude_functions.iter().map(String::as_str).collect();
    active_free_function_names(lib_rs_source, enabled_features)
        .into_iter()
        .filter(|name| !excluded.contains(name.as_str()))
        .filter(|name| {
            let camel = snake_to_camel(name);
            !contains_function_at_token_boundary(bridge_dart_source, &camel)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_function_names_lists_every_top_level_pub_fn() {
        let lib_rs = "\
use std::sync::Arc;

pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

pub async fn record_price(id: String, price_cents: i64) -> Result<(), String> {
    Ok(())
}

fn private_helper() {}
";
        assert_eq!(
            free_function_names(lib_rs),
            vec!["count_widgets".to_string(), "record_price".to_string()],
        );
    }

    #[test]
    fn missing_bridge_functions_reports_a_facade_function_absent_from_the_bridge() {
        let lib_rs = "\
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

pub fn record_price(id: String, price_cents: i64) -> Result<(), String> {
    Ok(())
}
";
        // The bridge only has `countWidgets` -- as if frb ran once before `record_price` was
        // added to the facade and never ran again afterward.
        let bridge_dart = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], None);
        assert_eq!(
            missing,
            vec!["record_price".to_string()],
            "record_price is declared in the facade but has no matching function in the bridge"
        );
    }

    #[test]
    fn missing_bridge_functions_is_empty_when_every_facade_function_is_bridged() {
        let lib_rs = "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n";
        let bridge_dart = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

        assert!(missing_bridge_functions(lib_rs, bridge_dart, &[], None).is_empty());
    }

    /// `dartfmt` wraps a long return type onto its own line, pushing the function name onto the
    /// line below preceded by a newline (and indentation), not a literal space -- the alef #191
    /// shape: a facade function that IS present and correctly bridged is reported missing purely
    /// because of how the bridge happened to be line-wrapped.
    #[test]
    fn missing_bridge_functions_finds_a_function_whose_name_is_wrapped_onto_its_own_line() {
        let lib_rs = "\
pub fn create_chunk_classification_definition_from_json(
    json: String,
) -> Result<ChunkClassificationDefinition, String> {
    todo!()
}
";
        // dartfmt wrapped the long return type onto its own line, so the function name starts a
        // fresh line with no preceding space -- only a preceding newline.
        let bridge_dart = "\
Future<ChunkClassificationDefinition>
createChunkClassificationDefinitionFromJson({required String json}) =>
    RustLib.instance.api.crateCreateChunkClassificationDefinitionFromJson(json: json);
";

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], None);
        assert_eq!(
            missing,
            Vec::<String>::new(),
            "the function is present and correctly bridged -- only line-wrapped -- and must not \
             be reported missing: {missing:?}"
        );
    }

    #[test]
    fn missing_bridge_functions_ignores_configured_exclusions() {
        let lib_rs = "\
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

pub fn internal_only(id: String) -> Result<(), String> {
    Ok(())
}
";
        // `internal_only` is deliberately stripped from the bridge post-frb (it never appears),
        // and that is expected -- excluding it must not show up as a coverage gap.
        let bridge_dart = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &["internal_only".to_string()], None);
        assert!(
            missing.is_empty(),
            "excluded function must not be reported missing: {missing:?}"
        );
    }

    /// The critical regression: a facade function gated behind a feature that is NOT in the
    /// enabled set must never be reported missing -- frb's own codegen never saw it either, so
    /// the bridge is not stale with respect to it. This is the shape of a real `alef all` failure
    /// this fix closes: several `#[cfg(feature = "...")]` functions a dart rust crate's manifest
    /// never enabled by default were reported as a stale bridge, even though frb correctly never
    /// saw them.
    #[test]
    fn missing_bridge_functions_ignores_a_facade_function_behind_an_inactive_cfg_gate() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        // The bridge has nothing for this function at all -- exactly what a correct frb run
        // produces when the gating feature is off.
        let bridge_dart = "";
        let enabled: HashSet<&str> = HashSet::new();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled));
        assert!(
            missing.is_empty(),
            "a facade function behind an inactive cfg gate must not be reported missing: {missing:?}"
        );
    }

    /// Negative control for the above: a facade function that is genuinely absent from the
    /// bridge -- either because it carries no cfg gate at all, or because its gate IS active --
    /// must still be reported missing. Without this control, a fix that simply stopped reporting
    /// every cfg-gated function (active or not) would pass the positive test above while quietly
    /// breaking the coverage check's entire purpose.
    #[test]
    fn missing_bridge_functions_still_reports_a_genuinely_missing_function_under_an_active_gate() {
        let lib_rs = "\
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

#[cfg(feature = \"premium-tier\")]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        // Neither function is bridged: `count_widgets` is ungated (always expected), and
        // `create_premium_backend_options_from_json`'s gate IS in the enabled set, so it is also
        // expected -- both must be reported.
        let bridge_dart = "";
        let enabled: HashSet<&str> = ["premium-tier"].into_iter().collect();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled));
        assert_eq!(
            missing,
            vec![
                "count_widgets".to_string(),
                "create_premium_backend_options_from_json".to_string(),
            ],
            "an ungated function and a function under an active gate must both still be reported \
             missing: {missing:?}"
        );
    }

    /// The real facade shape both positive tests above skip: `#[cfg(...)]` immediately followed
    /// by `#[frb]` before `pub fn` (see `frb_rewrite::cfg_gates::cfg_gated_free_functions`'s doc
    /// for why this specific shape matters -- it is what
    /// `backends::dart::templates::rust_from_json_bridge_fn.rs.jinja` always emits). Before that
    /// scanner learned to skip an intervening attribute, this gate was never recorded, so the
    /// function below was treated as ungated -- unconditionally expected in the bridge -- and
    /// reported missing even though its feature is off and frb correctly never emitted it.
    #[test]
    fn missing_bridge_functions_ignores_an_inactive_gate_behind_an_intervening_frb_attribute() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
#[frb]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        let bridge_dart = "";
        let enabled: HashSet<&str> = HashSet::new();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled));
        assert!(
            missing.is_empty(),
            "a gate followed by an intervening #[frb] attribute must still be recognized and, \
             being inactive, must not be reported missing: {missing:?}"
        );
    }

    /// Negative control for the test above: with the SAME `#[cfg(...)]` / `#[frb]` / `pub fn`
    /// shape, a function whose gate IS in the enabled set but is genuinely absent from the
    /// bridge must still be reported. Without this control, a fix that made the scanner treat
    /// every `#[frb]`-preceded gate as inactive (rather than correctly attaching and then
    /// evaluating it) would pass the positive test above while silencing this exact shape's
    /// coverage check entirely.
    #[test]
    fn missing_bridge_functions_still_reports_a_genuinely_missing_function_behind_an_intervening_frb_attribute() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
#[frb]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        let bridge_dart = "";
        let enabled: HashSet<&str> = ["premium-tier"].into_iter().collect();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled));
        assert_eq!(
            missing,
            vec!["create_premium_backend_options_from_json".to_string()],
            "a function under an active gate behind an intervening #[frb] attribute must still \
             be reported missing when absent from the bridge: {missing:?}"
        );
    }
}
