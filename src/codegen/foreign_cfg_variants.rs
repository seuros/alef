//! One WARN per foreign cfg-gated enum variant, for the whole generation run.
//!
//! A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
//! `#[cfg(...)]`. No generated binding crate declares a Cargo feature for it, so re-emitting the
//! gate verbatim is an `unexpected cfg condition value` error and every Rust-emitting backend
//! drops the variant instead. Each of those drops used to be its own `tracing::warn!`: fifteen
//! call sites -- eight backends (dart, extendr, ffi, napi, php, rustler, swift, wasm) plus the
//! three shared codegen modules (`conversions::enums`, `generators::enums`, `visitor_result`),
//! several of them once per conversion direction. A single variant therefore produced a wall of
//! WARN on every clean regen: the same fact re-reported once per (backend, direction, generator),
//! scaling with the backend count rather than with the number of variants a consumer can act on.
//!
//! The fan-out was the defect, not the level: `tracing-product-surface` lists skipped/unsupported
//! input under WARN, so demoting the fact would hide a real, actionable signal. Instead the fact
//! is reported once from here and the per-site detail stays at DEBUG, where it is still available
//! under `RUST_LOG=alef=debug` for anyone debugging codegen.
//!
//! Deduplication is by construction, not by a ledger: `ApiSurface` holds each enum once and each
//! variant once within it, so walking the surface a single time per run emits each
//! (enum, variant) fact exactly once with no mutable state to scope, share, or reset. The
//! fifteen emitting sites are pure `fn(&EnumDef, &EnumVariant, ..) -> String` helpers with no
//! reachable generation context, so threading a ledger to them would mean changing every
//! transitive caller up to `Backend::generate_bindings` in eight backends; a process-global
//! ledger is worse still, since it is settable once per process and would let one test's run
//! suppress another's. ~keep

use std::collections::{BTreeSet, HashSet};

use crate::codegen::cfg::{enabled_features_for_language, is_host_owned_rust_path};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, cfg_feature_satisfied};

fn universally_dropped_variant_references(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> BTreeSet<String> {
    let host_crates = host_crate_spellings(api, config, languages);
    if host_crates.is_empty() {
        return BTreeSet::new();
    }

    api.enums
        .iter()
        .filter(|enum_def| {
            host_crates
                .iter()
                .all(|host_crate| !is_host_owned_rust_path(host_crate, &enum_def.rust_path))
        })
        .flat_map(|enum_def| {
            enum_def
                .variants
                .iter()
                .filter(|variant| variant.cfg.is_some())
                .flat_map(|variant| {
                    [
                        format!("{}::{}", enum_def.name, variant.name),
                        format!("{}.{}", enum_def.name, variant.name),
                    ]
                })
        })
        .collect()
}

fn strip_unreachable_variant_doc_lines(value: &mut serde_json::Value, references: &BTreeSet<String>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                strip_unreachable_variant_doc_lines(value, references);
            }
        }
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                if name == "doc"
                    && let serde_json::Value::String(doc) = value
                {
                    *doc = doc
                        .split('\n')
                        .filter(|line| {
                            !references
                                .iter()
                                .any(|reference| contains_complete_variant_reference(line, reference))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                } else {
                    strip_unreachable_variant_doc_lines(value, references);
                }
            }
        }
        _ => {}
    }
}

fn contains_complete_variant_reference(line: &str, reference: &str) -> bool {
    line.match_indices(reference).any(|(start, _)| {
        let end = start + reference.len();
        let left_is_identifier = line[..start]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_alphanumeric() || character == '_');
        let right_is_identifier = line[end..]
            .chars()
            .next()
            .is_some_and(|character| character.is_alphanumeric() || character == '_');
        !left_is_identifier && !right_is_identifier
    })
}

/// Clone the extracted surface for generation while removing source-doc lines that advertise an
/// enum variant every requested backend necessarily omits.
///
/// The source IR remains intact for diagnostics and provenance. The projection is deliberately
/// limited to a variant whose enum is foreign under every requested backend's ownership spelling;
/// a mixed run where even one backend can expose the variant keeps the source documentation. ~keep
pub fn project_docs_without_unreachable_foreign_variants(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<ApiSurface> {
    let references = universally_dropped_variant_references(api, config, languages);
    if references.is_empty() {
        return Ok(api.clone());
    }

    let mut value = serde_json::to_value(api)?;
    strip_unreachable_variant_doc_lines(&mut value, &references);
    Ok(serde_json::from_value(value)?)
}

/// Every spelling of "the host crate" that a requested language's generator will classify enum
/// ownership against.
///
/// This is not one value: most backends ask `is_host_owned_rust_path` with
/// [`ResolvedCrateConfig::core_import_for_language`], but the Dart and Swift Rust-bridge
/// generators ask it with the plain crate name -- Dart's `source_crate_name` from
/// `config.name`, Swift's `source_crate` from `api.crate_name` -- ignoring both
/// `[crate] core_import` and their own `core_crate_override`. Both of those spellings are
/// contributed, since the two fields are separately sourced even though they normally agree.
/// That divergence predates this module and is deliberately not unified here: reproducing it is
/// what keeps this pre-pass from becoming a *second* answer to a question the backends already
/// answer for themselves. ~keep
fn host_crate_spellings(api: &ApiSurface, config: &ResolvedCrateConfig, languages: &[Language]) -> BTreeSet<String> {
    let mut spellings = BTreeSet::new();
    for &language in languages {
        match language {
            Language::Dart | Language::Swift => {
                spellings.insert(config.name.replace('-', "_"));
                spellings.insert(api.crate_name.replace('-', "_"));
            }
            other => {
                spellings.insert(config.core_import_for_language(other));
            }
        }
    }
    spellings
}

/// Report every reachable or indeterminate foreign-crate enum variant this run's generators will
/// drop for carrying a `#[cfg(...)]` no generated binding crate can declare.
///
/// Call once per generation run, before the per-language loop. The warning claims something
/// universal -- *every* generated binding crate drops this variant -- so it fires only when the
/// enum is foreign under every host-crate spelling the requested languages use. When the
/// spellings disagree (a `[crate] core_import` facade with Dart or Swift also requested) the
/// claim is not universal, so the fact stays at DEBUG on the backends that actually drop it
/// rather than being over-reported here as if it applied to all of them. A gate the canonical cfg
/// evaluator proves false under every requested language's effective features needs no warning:
/// the source variant is absent from every corresponding core build anyway. Indeterminate target
/// predicates remain warnings because absence cannot be proven at generation time. ~keep
pub fn warn_foreign_cfg_gated_variants(api: &ApiSurface, config: &ResolvedCrateConfig, languages: &[Language]) {
    let host_crates = host_crate_spellings(api, config, languages);
    if host_crates.is_empty() {
        return;
    }
    let enabled_features = languages
        .iter()
        .map(|&language| enabled_features_for_language(config, language))
        .collect::<Vec<_>>();
    let enabled_feature_refs = enabled_features
        .iter()
        .map(|features| features.iter().map(String::as_str).collect::<HashSet<_>>())
        .collect::<Vec<_>>();

    for enum_def in &api.enums {
        if host_crates
            .iter()
            .any(|host_crate| is_host_owned_rust_path(host_crate, &enum_def.rust_path))
        {
            continue;
        }
        let owning_crate = enum_def.rust_path.split("::").next().unwrap_or_default();
        for variant in &enum_def.variants {
            let Some(cfg) = variant.cfg.as_deref() else {
                continue;
            };
            if enabled_feature_refs
                .iter()
                .all(|features| !cfg_feature_satisfied(Some(cfg), features))
            {
                continue;
            }
            tracing::warn!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = cfg,
                owning_crate = owning_crate,
                "dropping a reachable or indeterminate foreign-crate enum variant from every \
                 generated binding: its #[cfg(...)] gate cannot be re-emitted in generated binding \
                 crates without an `unexpected cfg condition value` error. \
                 Either source-root the owning crate so alef controls its features, or exclude \
                 the enum. Run with RUST_LOG=alef=debug for the per-backend detail"
            );
        }
    }
}

#[cfg(test)]
mod tests;
