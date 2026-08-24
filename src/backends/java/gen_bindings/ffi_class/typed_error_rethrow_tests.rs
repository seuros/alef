//! The defect: a generated Java FFI method body closed with a bare `catch (Throwable e)` chain.
//!
//! `checkLastError()` reads the native error code and throws the exception subclass that code maps
//! to, carrying the real message the Rust side produced -- but it throws it from inside the
//! method's `try`. A chain whose first clause is `catch (Throwable e)` therefore caught the typed
//! exception it had just produced and replaced it with a fresh base exception whose message is the
//! placeholder "FFI call failed", demoting the real detail to a nested cause. Because every typed
//! exception extends the base exception, a preceding `catch (<Base>Exception e) { throw e; }`
//! restores it.
//!
//! The visitor path carried that guard on its outer chain only. Its inner operation `catch` -- the
//! one that actually encloses the `checkLastError()` calls -- re-wrapped first, so the outer guard
//! never saw a typed exception and the path was broken all the same. ~keep

use crate::core::config::{BridgeBinding, HostCapsuleTypeConfig, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use std::collections::{HashMap, HashSet};

use super::gen_main_class;

const BASE_EXCEPTION: &str = "SampleException";
const THROWABLE_CATCH: &str = "catch (Throwable e) {";
const PLACEHOLDER_WRAP: &str = "new SampleException(\"FFI call failed\"";

/// One per emitting path that closes a body with the placeholder wrap: three regular sync methods
/// (`returns.rs`), one capsule-returning sync method (`sync_functions.rs`), and the visitor
/// internal method's inner operation chain plus its outer chain (`conversion_internals.rs`).
const EXPECTED_WRAPPING_CLAUSES: usize = 6;

/// The two shapes the shared helper emits: the plain method chain and the visitor operation chain
/// that also records the escaping exception for the `finally` cleanup block.
const ALLOWED_GUARDS: [&str; 2] = [
    "catch (SampleException e) { throw e; } ",
    "catch (SampleException e) { operationFailure = e; throw e; } ",
];

#[test]
fn every_ffi_catch_chain_rethrows_typed_exceptions_before_wrapping() {
    let generated = generate_main_class();
    let collapsed = collapse_whitespace(&generated);

    let mut wrapping_clauses = 0usize;
    for (index, _) in collapsed.match_indices(THROWABLE_CATCH) {
        let body_start = index + THROWABLE_CATCH.len();
        let body_end = collapsed[body_start..]
            .find('}')
            .map_or(collapsed.len(), |offset| body_start + offset);
        if !collapsed[body_start..body_end].contains(PLACEHOLDER_WRAP) {
            continue;
        }
        wrapping_clauses += 1;

        let preceding = &collapsed[..index];
        let guard = preceding
            .rfind("catch (SampleException e) {")
            .map(|start| &preceding[start..]);
        assert!(
            guard.is_some_and(|clause| ALLOWED_GUARDS.contains(&clause)),
            "a `catch (Throwable e)` clause that re-wraps into {BASE_EXCEPTION}(\"FFI call failed\") \
             must be preceded directly by a typed rethrow clause, otherwise the exception \
             `checkLastError()` throws loses its native message; found guard {guard:?} in:\n{generated}"
        );
    }

    assert_eq!(
        wrapping_clauses, EXPECTED_WRAPPING_CLAUSES,
        "sanity: the fixture must still exercise every path that closes a body with the \
         placeholder wrap -- sync, capsule, visitor operation and visitor outer\n{generated}"
    );
}

/// The typed exception must be reachable at all: `checkLastError()` is the only place that turns a
/// native error code into a message-carrying subclass, and it must do so from inside the `try` the
/// guarded chain closes.
#[test]
fn check_last_error_throws_message_carrying_subclasses_inside_the_guarded_try() {
    let generated = generate_main_class();

    assert!(
        generated.contains("case 2 -> throw new CoreErrorException(msg);"),
        "checkLastError() must map an error code onto a typed subclass carrying the native message:\n{generated}"
    );
    assert!(
        generated.contains("private static void checkLastError() throws Throwable {"),
        "checkLastError() must stay a checked-throwing helper called from inside method bodies:\n{generated}"
    );
}

/// The async wrapper delegates to the sync method and is required to surface failures as
/// `CompletionException` -- the `CompletableFuture` contract. It must never introduce a second
/// placeholder wrap of its own, which would bury the typed exception one level deeper again.
#[test]
fn async_wrapper_delegates_without_re_wrapping_into_the_base_exception() {
    let generated = generate_main_class();
    let body = async_wrapper_body(&generated);

    assert!(
        body.contains("throw new CompletionException(e);"),
        "async wrapper must surface failures through CompletionException:\n{body}"
    );
    assert!(
        !body.contains(PLACEHOLDER_WRAP),
        "async wrapper must not re-wrap the sync method's typed exception into the base \
         exception -- the typed exception has to stay the direct cause:\n{body}"
    );
}

fn async_wrapper_body(generated: &str) -> &str {
    let start = generated
        .find("public static CompletableFuture<BatchOutcome> fetchPageAsync()")
        .expect("fixture must generate an async wrapper");
    let rest = &generated[start..];
    let end = rest
        .find("\n    }\n")
        .map_or(rest.len(), |offset| offset + "\n    }\n".len());
    &rest[..end]
}

fn collapse_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn generate_main_class() -> String {
    let (api, config, capsule_types) = typed_error_surface();
    gen_main_class(
        &api,
        &config,
        "dev.sample",
        "Sample",
        "smp",
        &HashSet::new(),
        &HashSet::new(),
        true,
        &capsule_types,
    )
}

/// A surface that reaches every generated FFI body shape at once: a plain sync function, a
/// capsule-returning sync function, an async function, and a visitor-bridged function.
fn typed_error_surface() -> (ApiSurface, ResolvedCrateConfig, HashMap<String, HostCapsuleTypeConfig>) {
    let api = ApiSurface {
        functions: vec![
            FunctionDef {
                name: "run_batch".to_string(),
                rust_path: "sample::run_batch".to_string(),
                params: vec![ParamDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("BatchOutcome".to_string()),
                error_type: Some("SampleError".to_string()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "load_grammar".to_string(),
                rust_path: "sample::load_grammar".to_string(),
                return_type: TypeRef::Named("Grammar".to_string()),
                error_type: Some("SampleError".to_string()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "fetch_page".to_string(),
                rust_path: "sample::fetch_page".to_string(),
                return_type: TypeRef::Named("BatchOutcome".to_string()),
                is_async: true,
                error_type: Some("SampleError".to_string()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "walk".to_string(),
                rust_path: "sample::walk".to_string(),
                params: vec![
                    ParamDef {
                        name: "source".to_string(),
                        ty: TypeRef::String,
                        ..ParamDef::default()
                    },
                    ParamDef {
                        name: "options".to_string(),
                        ty: TypeRef::Named("WalkOptions".to_string()),
                        ..ParamDef::default()
                    },
                ],
                return_type: TypeRef::Named("BatchOutcome".to_string()),
                error_type: Some("SampleError".to_string()),
                ..FunctionDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "SampleWalker".to_string(),
            type_alias: Some("SampleWalkerHandle".to_string()),
            param_name: Some("renderer".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some("WalkOptions".to_string()),
            options_field: Some("renderer".to_string()),
            context_type: Some("SampleContext".to_string()),
            result_type: Some("BatchOutcome".to_string()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };

    let mut capsule_types = HashMap::new();
    capsule_types.insert(
        "Grammar".to_string(),
        HostCapsuleTypeConfig {
            host_type: "dev.sample.host.Grammar".to_string(),
            package: "dev.sample:host".to_string(),
            package_version: "1.0.0".to_string(),
            construct_expr: "new Grammar({ptr})".to_string(),
            ..Default::default()
        },
    );

    (api, config, capsule_types)
}
