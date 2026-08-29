//! Resolve the synthetic parameter a streaming adapter's flattened Go wrapper implies for a
//! call, so `snippet.rs`'s typed-literal rendering can consult it instead of re-deriving it.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ParamDef, TypeDef};
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::config::CallConfig;

/// The DTO field `gen_bindings::functions::gen_adapter_wrapper` decomposes into a scalar Go
/// parameter for `call`, expressed as a synthetic single-element `target_params` list, when
/// `call` resolves to a streaming adapter that flattens one.
///
/// `recipe.target_params` (via `CallIr::signature`) has no notion of this decomposition: it
/// answers with the real Rust signature, which for a flattening adapter is the *wrapping*
/// request DTO the Go binding never actually takes as a parameter -- not the single field it
/// declares instead. Asking [`crate::backends::go::adapter_flattened_field`] directly, the same
/// function `gen_adapter_wrapper` renders from, keeps `render_snippet_body`'s typed-literal
/// rendering answering the question the Go backend actually answered, instead of re-deriving
/// (and risking disagreeing with) its own copy of the same test -- which is how the Java/C#
/// streaming-request bug happened in the first place.
///
/// `None` means the caller must fall back to its own IR-derived `target_params`: either `call`
/// is not a streaming adapter at all, or it is one that does *not* flatten. A non-flattening
/// adapter's configured `[[adapters]] params` already mirror its real extracted Rust signature
/// one-for-one -- the generated FFI shim binds each by name (see
/// `adapters::streaming::core_let_bindings`) -- so the caller's IR-derived `target_params`
/// already agrees with `gen_adapter_wrapper` there without any help from this function.
///
/// Returned as an owned `Vec` rather than a `TargetParams` because `TargetParams::Known`
/// borrows: pair this with [`target_params_or`], which the caller invokes on a binding it
/// keeps alive for as long as `target_params` is in scope. ~keep
pub(super) fn flattened_stream_params(
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    lang: &str,
    call: &CallConfig,
) -> Option<Vec<ParamDef>> {
    let lookup_name = call.core_lookup_name(lang)?;
    let adapter = config.adapters.iter().find(|adapter| adapter.name == lookup_name)?;
    let field = crate::backends::go::adapter_flattened_field(adapter, type_defs)?;
    Some(vec![ParamDef {
        name: field.name.clone(),
        ty: field.ty.clone(),
        optional: field.optional,
        ..ParamDef::default()
    }])
}

/// Resolves `target_params` from a [`flattened_stream_params`] result: `Known` when it
/// produced one, otherwise `fallback()` (typically the caller's own `recipe.target_params`). ~keep
pub(super) fn target_params_or<'a>(
    flattened: &'a Option<Vec<ParamDef>>,
    fallback: impl FnOnce() -> TargetParams<'a>,
) -> TargetParams<'a> {
    match flattened.as_deref() {
        Some(params) => TargetParams::Known(params),
        None => fallback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern};
    use crate::core::ir::{FieldDef, TypeRef};

    fn streaming_adapter(name: &str, params: Vec<AdapterParam>, request_type: Option<&str>) -> AdapterConfig {
        AdapterConfig {
            name: name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: format!("sample::CrawlEngine::{name}"),
            params,
            returns: None,
            error_type: None,
            owner_type: Some("CrawlEngineHandle".to_string()),
            item_type: Some("sample::CrawlEvent".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: request_type.map(str::to_string),
            skip_languages: Vec::new(),
        }
    }

    fn adapter_param(name: &str, ty: &str) -> AdapterParam {
        AdapterParam {
            name: name.to_string(),
            ty: ty.to_string(),
            optional: false,
        }
    }

    fn dto(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields,
            ..TypeDef::default()
        }
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn call_named(function: &str) -> CallConfig {
        CallConfig {
            function: function.to_string(),
            ..CallConfig::default()
        }
    }

    fn config_with(adapters: Vec<AdapterConfig>) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            adapters,
            ..ResolvedCrateConfig::default()
        }
    }

    #[test]
    fn resolves_the_flattened_field_for_a_matching_streaming_call() {
        let config = config_with(vec![streaming_adapter(
            "crawl_stream",
            vec![adapter_param("request", "CrawlRequest")],
            Some("sample::CrawlRequest"),
        )]);
        let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

        let params = flattened_stream_params(&config, &types, "go", &call_named("crawl_stream"));
        let names_and_types: Option<Vec<(String, TypeRef)>> =
            params.map(|params| params.into_iter().map(|p| (p.name, p.ty)).collect());

        assert_eq!(names_and_types, Some(vec![("url".to_string(), TypeRef::String)]));
    }

    #[test]
    fn none_when_no_adapter_matches_the_call() {
        let config = config_with(vec![streaming_adapter(
            "crawl_stream",
            vec![adapter_param("request", "CrawlRequest")],
            Some("sample::CrawlRequest"),
        )]);
        let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

        assert!(flattened_stream_params(&config, &types, "go", &call_named("other_call")).is_none());
    }

    #[test]
    fn none_when_the_matching_adapter_does_not_flatten() {
        let config = config_with(vec![streaming_adapter(
            "crawl_stream",
            vec![adapter_param("url", "String"), adapter_param("depth", "u32")],
            Some("sample::CrawlRequest"),
        )]);
        let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

        assert!(flattened_stream_params(&config, &types, "go", &call_named("crawl_stream")).is_none());
    }

    #[test]
    fn none_when_the_call_names_no_function() {
        let config = config_with(vec![streaming_adapter(
            "crawl_stream",
            vec![adapter_param("request", "CrawlRequest")],
            Some("sample::CrawlRequest"),
        )]);
        let types = vec![dto("CrawlRequest", vec![field("url", TypeRef::String)])];

        assert!(flattened_stream_params(&config, &types, "go", &call_named("")).is_none());
    }

    #[test]
    fn target_params_or_prefers_the_flattened_list_over_the_fallback() {
        let flattened = Some(vec![ParamDef {
            name: "url".to_string(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }]);

        let resolved = target_params_or(&flattened, || panic!("fallback must not run when flattened is Some"));

        assert_eq!(resolved.known().map(|params| params.len()), Some(1));
    }

    #[test]
    fn target_params_or_runs_the_fallback_when_nothing_flattened() {
        let flattened: Option<Vec<ParamDef>> = None;

        let resolved = target_params_or(&flattened, || TargetParams::Unresolvable);

        assert!(resolved.known().is_none());
    }
}
