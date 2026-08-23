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

use super::cfg_gates::free_pub_fn_name;
use super::text_transformations::{contains_function_at_token_boundary, snake_to_camel};

/// Names of every top-level (column 0) `pub fn` / `pub async fn` free function declared in
/// `lib_rs_source`, in declaration order.
///
/// This is the full set flutter_rust_bridge should bridge from the facade -- unlike
/// [`super::cfg_gates::cfg_gated_free_functions`], which only returns the `#[cfg(...)]`-gated
/// subset.
pub fn free_function_names(lib_rs_source: &str) -> Vec<String> {
    lib_rs_source.lines().filter_map(free_pub_fn_name).collect()
}

/// Names (as declared in `lib_rs_source`, snake_case) of facade free functions that have no
/// matching function in `bridge_dart_source`.
///
/// `exclude_functions` are removed from consideration: those are deliberately stripped from the
/// bridge post-frb by [`super::text_transformations::filter_excluded_functions`] and are expected
/// to be permanently absent, independent of whether the bridge is otherwise fresh.
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
) -> Vec<String> {
    let excluded: std::collections::HashSet<&str> = exclude_functions.iter().map(String::as_str).collect();
    free_function_names(lib_rs_source)
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

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[]);
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

        assert!(missing_bridge_functions(lib_rs, bridge_dart, &[]).is_empty());
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

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[]);
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

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &["internal_only".to_string()]);
        assert!(
            missing.is_empty(),
            "excluded function must not be reported missing: {missing:?}"
        );
    }
}
