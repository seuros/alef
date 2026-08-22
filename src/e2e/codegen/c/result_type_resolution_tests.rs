//! Tests for the C backend's call-result type resolution.
//!
//! Split out of `c.rs`: the inline module was ~700 of that file's 3,030 lines. `c.rs` already
//! keeps `snippet_tests` inline, so this takes the largest block first rather than restructuring
//! the whole file.

use super::*;
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};

fn call_named(function: &str) -> CallConfig {
    CallConfig {
        function: function.to_string(),
        ..CallConfig::default()
    }
}

fn function_returning(name: &str, return_type: TypeRef, error_type: Option<&str>) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        error_type: error_type.map(str::to_string),
        ..FunctionDef::default()
    }
}

fn method_returning(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        error_type: Some("String".to_string()),
        ..MethodDef::default()
    }
}

fn type_with_methods(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        methods,
        ..TypeDef::default()
    }
}

fn ir_functions(functions: &[FunctionDef]) -> CallIr<'_> {
    CallIr {
        functions,
        type_defs: &[],
    }
}

fn ir_types(type_defs: &[TypeDef]) -> CallIr<'_> {
    CallIr {
        functions: &[],
        type_defs,
    }
}

/// The case that was silently wrong. The extractor splits `Result<T, E>` into
/// `return_type = T` plus a separate `error_type`, so a fallible
/// `pub fn complete(..) -> Result<CompletionResponse, String>` must resolve to
/// `CompletionResponse` — never to `Complete`, the PascalCased call name, which is not a
/// type at all and disables the nested-field walk that reads it.
#[test]
fn should_resolve_a_fallible_functions_result_type_to_its_ok_type() {
    let functions = vec![function_returning(
        "complete",
        TypeRef::Named("CompletionResponse".to_string()),
        Some("String"),
    )];

    assert_eq!(
        resolve_ir_result_type(&call_named("complete"), "c", ir_functions(&functions)),
        Some("CompletionResponse".to_string())
    );
}

/// Control: the `Optional(Named)` shape the previous one-level match already handled must
/// keep resolving to the same name.
#[test]
fn should_resolve_an_optional_named_return_type_unchanged() {
    let functions = vec![function_returning(
        "find_model",
        TypeRef::Optional(Box::new(TypeRef::Named("Model".to_string()))),
        Some("String"),
    )];

    assert_eq!(
        resolve_ir_result_type(&call_named("find_model"), "c", ir_functions(&functions)),
        Some("Model".to_string())
    );
}

/// `Result<Vec<Model>, E>` answered `None` under the one-level match and fell through to
/// the call-name fallback, even though the sibling `named_type` in this very module already
/// unwrapped `Vec`.
#[test]
fn should_resolve_through_a_collection_return_type() {
    let functions = vec![function_returning(
        "list_models",
        TypeRef::Vec(Box::new(TypeRef::Named("Model".to_string()))),
        Some("String"),
    )];

    assert_eq!(
        resolve_ir_result_type(&call_named("list_models"), "c", ir_functions(&functions)),
        Some("Model".to_string())
    );
}

/// A return type with no named type in it has no result type to name; the caller's
/// fallback, not a wrong name, is the right answer here.
#[test]
fn should_not_invent_a_result_type_for_an_unnamed_return() {
    let functions = vec![function_returning("ping", TypeRef::Unit, None)];

    assert_eq!(
        resolve_ir_result_type(&call_named("ping"), "c", ir_functions(&functions)),
        None
    );
}

/// The derived name stays load-bearing for callers that genuinely have no IR — this
/// module's own cases, and the visitor call sites. With `type_defs` empty there is nothing
/// any IR-keyed check could have consulted anyway, so this must stay renderable rather than
/// becoming a hard failure.
#[test]
fn should_fall_back_to_the_pascal_cased_call_name_without_ir_functions() {
    assert_eq!(
        resolve_ir_result_type(&call_named("complete"), "c", CallIr::default()),
        None
    );
    assert_eq!(
        unresolved_result_type_name(&call_named("complete"), "c", CallIr::default(), None)
            .require()
            .expect("a no-IR caller must still render"),
        "Complete",
        "the no-IR arm must stay, and its output must stay the documented shape"
    );
}

/// The error fires when a call genuinely has an unresolvable name AND no config
/// declaration explains why — the authoring gap this arm exists to catch. The IR is
/// deliberately non-empty (a real crate to consult, so `ir.is_absent()` is false) but does
/// not name this call, distinguishing this from the "no IR at all" debug case above.
#[tracing_test::traced_test]
#[test]
fn unresolved_result_type_name_refuses_an_unresolvable_call_with_no_declaration() {
    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];

    let result_type = unresolved_result_type_name(&call_named("mystery_call"), "c", ir_functions(&functions), None);

    let error = result_type
        .require()
        .expect_err("an unresolvable result type must not hand back a name");
    assert!(
        error.to_string().contains("mystery_call"),
        "the error must name the call an operator has to fix: {error}"
    );
    assert!(
        !error.to_string().contains("MysteryCall"),
        "the error must not leak the PascalCased call name as if it were a type: {error}"
    );
    assert!(
        logs_contain("no real type to name"),
        "an unresolvable call with no result_is_bytes/simple/json_struct declaration must say \
         so when it is classified, not only when an emission path happens to ask for the name"
    );
}

/// Negative control / regression for the false alarm this fix addresses: a call whose
/// result is declared `result_is_bytes` under the C override has no named type to set and
/// no nested field to verify, so the error's suggested fix ("set `result_type`") is
/// meaningless here — and it must not fire. Mirrors the real bug report's exact shape:
/// `[crates.e2e.calls.speech.overrides.c] result_is_bytes = true` against a call whose
/// IR-side type (`bytes::Bytes`) has no `pub struct`/`pub enum` in core at all.
#[tracing_test::traced_test]
#[test]
fn unresolved_result_type_name_stays_silent_for_a_declared_bytes_result() {
    use crate::e2e::config::CallOverride;

    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];
    let mut call = call_named("speech");
    call.overrides.insert(
        "c".to_string(),
        CallOverride {
            result_is_bytes: true,
            ..CallOverride::default()
        },
    );

    unresolved_result_type_name(&call, "c", ir_functions(&functions), None)
        .require()
        .expect("a declared-bytes result must still render");

    assert!(
        !logs_contain("no real type to name"),
        "a declared-bytes result has no named type and no nested field to check; failing \
         generation and telling the operator to set `result_type` is a false alarm"
    );
    assert!(
        logs_contain("carries no named fields"),
        "apparatus check: the debug-level explanation must actually fire, or the silence \
         above proves nothing about which branch ran"
    );
}

/// The call-level `result_is_simple` flag — identical semantics to `result_is_bytes`: no
/// named struct, nothing to verify — must suppress the failure too, not just the
/// byte-buffer case.
#[tracing_test::traced_test]
#[test]
fn unresolved_result_type_name_stays_silent_for_a_call_level_simple_result() {
    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];
    let mut call = call_named("ping");
    call.result_is_simple = true;

    unresolved_result_type_name(&call, "c", ir_functions(&functions), None)
        .require()
        .expect("a declared-simple result must still render");

    assert!(!logs_contain("no real type to name"));
    assert!(logs_contain("carries no named fields"));
}

/// The Zig-only `result_is_json_struct` override flag makes the same "opaque, verified
/// structurally, not by named-field lookup" declaration and belongs in the same
/// suppression set as `result_is_bytes` / `result_is_simple`.
#[tracing_test::traced_test]
#[test]
fn unresolved_result_type_name_stays_silent_for_a_declared_json_struct_result() {
    use crate::e2e::config::CallOverride;

    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];
    let mut call = call_named("extract");
    call.overrides.insert(
        "c".to_string(),
        CallOverride {
            result_is_json_struct: true,
            ..CallOverride::default()
        },
    );

    unresolved_result_type_name(&call, "c", ir_functions(&functions), None)
        .require()
        .expect("a declared-json-struct result must still render");

    assert!(!logs_contain("no real type to name"));
    assert!(logs_contain("carries no named fields"));
}

/// Fourth case this module documents: `register_fn` / `unregister_fn` / `clear_fn` on
/// `[[crates.trait_bridges]]` name FFI exports the backend generates itself
/// (`src/backends/ffi/trait_bridge/registration.rs`), never core IR functions -- so a
/// trait-bridge registry call is unresolvable against `ir` by construction, not because an
/// author forgot to export something. That is not the authoring gap `Unresolvable` exists to
/// catch, and every consumer that uses trait bridges would otherwise fail its first regen.
#[tracing_test::traced_test]
#[test]
fn unresolved_result_type_name_stays_silent_for_a_trait_bridge_registry_call() {
    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];
    let call = call_named("clear_sample_validators");

    let classified = unresolved_result_type_name(&call, "c", ir_functions(&functions), Some("clear_sample_validator"));
    let result_type = classified
        .require()
        .expect("a trait-bridge registry call must still render, not bail generation");

    assert_eq!(
        result_type, "ClearSampleValidator",
        "the classified name must come from the derived registry identity, not the (here \
         empty) base `function` -- an empty derivation is the exact `{{prefix}}__free` \
         regression this guards against"
    );
    assert!(!logs_contain("no real type to name"));
    assert!(logs_contain("no core IR counterpart"));
}

/// Control for the test above: an ORDINARY call absent from the IR, with no
/// result_is_bytes/simple/json_struct declaration and no trait-bridge identity (`None`),
/// must still bail. This is the regression guard for the fix itself: a change that
/// classified every unmatched call as `Unverified` -- rather than specifically a
/// trait-bridge registry call -- would pass the positive test above while silently
/// disabling the authoring-gap guard for every ordinary call, which is the exact failure
/// mode the preceding lane's `Unresolvable` arm was introduced to catch. ~keep
#[tracing_test::traced_test]
#[test]
fn unresolved_result_type_name_still_refuses_an_unresolvable_call_when_no_trait_bridge_identity_matches() {
    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];
    let call = call_named("another_mystery_call");

    let result_type = unresolved_result_type_name(&call, "c", ir_functions(&functions), None);

    result_type
        .require()
        .expect_err("an ordinary unresolvable call must still fail generation");
    assert!(logs_contain("no real type to name"));
}

/// End-to-end regression for the real over-reach, through `resolve_fixture_call_info` --
/// the path `render_test_file` (the main e2e test-file generator, not the docs-site
/// snippet path) actually calls. A trait-bridge registry call whose `CallConfig` is
/// unresolvable against the core IR and declares no result_is_bytes/simple/json_struct must
/// resolve without bailing for all three registry operations. Before this fix, every one of
/// these classified as `Unresolvable` and turned a consumer's first regen after adding a
/// trait bridge into a hard generation failure. ~keep
#[test]
fn resolve_fixture_call_info_does_not_bail_for_any_trait_bridge_registry_operation() {
    let functions = vec![function_returning(
        "unrelated",
        TypeRef::Named("Unrelated".to_string()),
        Some("String"),
    )];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "SampleValidator".into(),
            register_fn: Some("register_sample_validator".into()),
            unregister_fn: Some("unregister_sample_validator".into()),
            clear_fn: Some("clear_sample_validators".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };

    for identity in [
        "register_sample_validator",
        "unregister_sample_validator",
        "clear_sample_validators",
    ] {
        let fixture = Fixture {
            id: identity.into(),
            call: Some(identity.into()),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(identity.into(), CallConfig::default());

        let info = resolve_fixture_call_info(&fixture, &e2e, &config, "c", ir_functions(&functions));

        info.result_type_name
            .require()
            .unwrap_or_else(|error| panic!("{identity} must not bail generation: {error}"));
    }
}

/// A trait-bridge `clear` export returns an `i32` status (`clear_fn.jinja`), so the emitted
/// C must bind it to `int32_t` and free NOTHING.
///
/// This assertion replaces one that required a `clear_sample_validator_free(` call. That
/// earlier pin was a partial reading of the same defect: it correctly rejected the
/// degenerate `{prefix}__free` an empty derived name produced, and then demanded a
/// non-degenerate spelling of a free that must not be emitted at all. Freeing a status
/// integer as an alef `Box` corrupts the heap whatever the symbol is called, so the
/// absence of ANY free -- not the well-formedness of one -- is the property worth pinning.
///
/// Every clause below examines a different way to get this wrong: the bound type (an `i32`
/// bound to `{PREFIX}AlefHandle` is the original defect), the polarity of the status check
/// (`0` is success here, the inverse of the null-handle convention every other C path
/// uses), and the presence of any cleanup call. ~keep
#[test]
fn trait_bridge_clear_snippet_binds_an_int32_status_and_frees_nothing() {
    let fixture = Fixture {
        id: "clear_sample_validators".into(),
        description: "Clear registered sample validators".into(),
        call: Some("clear_sample_validators".into()),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".into(),
            ..Default::default()
        }],
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    let mut call = CallConfig::default();
    call.overrides.insert(
        "c".to_string(),
        crate::e2e::config::CallOverride {
            function: Some("clear_sample_validators".to_string()),
            ..crate::e2e::config::CallOverride::default()
        },
    );
    e2e.calls.insert("clear_sample_validators".into(), call);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "SampleValidator".into(),
            clear_fn: Some("clear_sample_validators".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let functions = [crate::core::ir::FunctionDef {
        name: "unrelated".into(),
        return_type: TypeRef::Named("Unrelated".into()),
        ..crate::core::ir::FunctionDef::default()
    }];

    let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &functions)
        .expect("a trait-bridge clear call absent from the IR must still render");

    // Matched up to the opening paren, not through the argument list: this fixture names
    // the call through a per-language `function` override, and the `extra_args` fallback
    // that appends the mandatory trailing `out_error` is gated on an EMPTY `function_name`
    // (`resolve_fixture_call_info`), so the arguments here are a separate, still-open
    // defect on the argument axis. Spelling them out would pin it as correct. The
    // out_error argument is covered by
    // `trait_bridge_out_error_arg_comes_from_extra_args_not_from_a_null_fixture_input`. ~keep
    assert!(
        rendered.contains("int32_t result = sample_clear_sample_validators("),
        "a registry export returns an i32 status and must be bound as one: {rendered}"
    );
    assert!(
        !rendered.contains("AlefHandle result"),
        "an i32 status must never be bound to an opaque handle type: {rendered}"
    );
    assert!(
        !rendered.contains("_free("),
        "a status code owns nothing; any `_free` here passes a non-pointer to a function \
         that frees an alef `Box`: {rendered}"
    );
}

/// The e2e test-file counterpart of the snippet test above -- `render_test_file` keeps the
/// assertions a documentation snippet strips, so it is the only place the status CHECK is
/// observable. `0` is success for these exports (`clear_fn.jinja` returns `0` on success
/// and `1` on failure), which inverts the `!= 0` convention every opaque-handle path in
/// this backend uses; a test that only checked for "some assertion mentioning result"
/// would pass on the inverted one. ~keep
#[test]
fn trait_bridge_clear_e2e_test_asserts_a_zero_status_and_emits_no_cleanup() {
    let fixture = Fixture {
        id: "clear_sample_validators".into(),
        description: "Clear registered sample validators".into(),
        call: Some("clear_sample_validators".into()),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".into(),
            ..Default::default()
        }],
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    let mut call = CallConfig::default();
    call.overrides.insert(
        "c".to_string(),
        // Already prefixed: unlike the snippet path, `render_test_file` uses
        // `function_name` verbatim -- the `c` override is documented as carrying the full
        // ABI symbol. ~keep
        crate::e2e::config::CallOverride {
            function: Some("sample_clear_sample_validators".to_string()),
            ..crate::e2e::config::CallOverride::default()
        },
    );
    e2e.calls.insert("clear_sample_validators".into(), call);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "SampleValidator".into(),
            clear_fn: Some("clear_sample_validators".into()),
            ..Default::default()
        }],
        ..ResolvedCrateConfig::default()
    };
    let functions = [crate::core::ir::FunctionDef {
        name: "unrelated".into(),
        return_type: TypeRef::Named("Unrelated".into()),
        ..crate::core::ir::FunctionDef::default()
    }];
    let resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    let rendered = render_test_file(
        "plugins",
        &[&fixture],
        "sample.h",
        "sample",
        "result",
        &e2e,
        "c",
        &resolver,
        &config,
        &[],
        &[],
        &[],
        CallIr {
            functions: &functions,
            type_defs: &[],
        },
    )
    .expect("a trait-bridge clear call renders an e2e test function");

    assert!(
        rendered.contains("int32_t result = sample_clear_sample_validators("),
        "the e2e path must bind the status as an int32_t too: {rendered}"
    );
    assert!(
        rendered.contains("assert(result == 0 && \"expected call to succeed\");"),
        "success for a registry export is a ZERO status, not a non-null handle: {rendered}"
    );
    assert!(
        !rendered.contains("_free("),
        "the e2e test must not free a status code: {rendered}"
    );
}

/// The other half of the gap this module documents: `ApiSurface::functions` holds free
/// `pub fn`s only, so a call naming an inherent or trait method — a consumer's `chat`, the
/// motivating case — is absent from it no matter how well `functions` is threaded. The
/// method lives on `TypeDef::methods`, which the C generator already had in hand.
#[test]
fn should_resolve_a_method_call_from_the_type_registry() {
    let type_defs = vec![type_with_methods(
        "LlmClient",
        vec![method_returning("chat", TypeRef::Named("ChatResponse".to_string()))],
    )];

    assert_eq!(
        resolve_ir_result_type(&call_named("chat"), "c", ir_types(&type_defs)),
        Some("ChatResponse".to_string())
    );
}

/// A free function of the same name is the unambiguous answer and must win, so adding
/// method lookup cannot change what an already-resolving call resolves to.
#[test]
fn should_prefer_a_free_function_over_a_same_named_method() {
    let functions = vec![function_returning(
        "chat",
        TypeRef::Named("FreeFunctionResponse".to_string()),
        Some("String"),
    )];
    let type_defs = vec![type_with_methods(
        "LlmClient",
        vec![method_returning("chat", TypeRef::Named("ChatResponse".to_string()))],
    )];
    let ir = CallIr {
        functions: &functions,
        type_defs: &type_defs,
    };

    assert_eq!(
        resolve_ir_result_type(&call_named("chat"), "c", ir),
        Some("FreeFunctionResponse".to_string())
    );
}

/// Two types declaring `chat` with different return types give the IR no single answer.
/// Guessing one would be worse than the fallback, because a wrong-but-plausible IR type
/// name switches `ensure_leaf_field_exists` ON against the wrong parent and fails
/// generation with a diagnostic pointing at the wrong type. Decline instead.
#[test]
fn should_decline_an_ambiguous_method_name() {
    let type_defs = vec![
        type_with_methods(
            "LlmClient",
            vec![method_returning("chat", TypeRef::Named("ChatResponse".to_string()))],
        ),
        type_with_methods(
            "MockClient",
            vec![method_returning("chat", TypeRef::Named("MockResponse".to_string()))],
        ),
    ];

    assert_eq!(
        resolve_ir_result_type(&call_named("chat"), "c", ir_types(&type_defs)),
        None
    );
}

/// The same method reached through both an inherent impl and a trait impl is listed twice
/// with the same signature. That is not ambiguity — declining there would leave the common
/// case unresolved for no reason.
#[test]
fn should_resolve_a_method_duplicated_with_an_identical_signature() {
    let method = MethodDef {
        params: vec![ParamDef {
            name: "request".to_string(),
            ty: TypeRef::Named("ChatRequest".to_string()),
            ..ParamDef::default()
        }],
        ..method_returning("chat", TypeRef::Named("ChatResponse".to_string()))
    };
    let type_defs = vec![
        type_with_methods("LlmClient", vec![method.clone()]),
        type_with_methods("OpenAiClient", vec![method]),
    ];

    assert_eq!(
        resolve_ir_result_type(&call_named("chat"), "c", ir_types(&type_defs)),
        Some("ChatResponse".to_string())
    );
}

/// End to end through `resolve_call_info`: with the IR threaded, the resolved type wins
/// over the fallback; with no IR, the fallback still applies.
#[test]
fn should_prefer_the_resolved_result_type_over_the_call_name_fallback() {
    let functions = vec![function_returning(
        "complete",
        TypeRef::Named("CompletionResponse".to_string()),
        Some("String"),
    )];

    assert_eq!(
        resolve_call_info(&call_named("complete"), "c", ir_functions(&functions), None)
            .result_type_name
            .require()
            .expect("a call the IR names must resolve"),
        "CompletionResponse"
    );
    assert_eq!(
        resolve_call_info(&call_named("complete"), "c", CallIr::default(), None)
            .result_type_name
            .require()
            .expect("a no-IR caller must still render"),
        "Complete"
    );
}

/// Task 4: an operator-set `result_type` override short-circuits BOTH the IR lookup and
/// `unresolved_result_type_name` — so a primitive/pointer spelling there (a call override
/// typo, or copy-pasting `raw_c_result_type`'s valid values into the wrong field) reached
/// no diagnostic at all before this, unlike the unresolvable-call case one test above,
/// which now fails generation outright. This is the positive case: the warning must fire.
#[tracing_test::traced_test]
#[test]
fn resolve_call_info_warns_when_result_type_override_is_a_primitive_spelling() {
    use crate::e2e::config::CallOverride;

    let mut call = call_named("speech");
    call.overrides.insert(
        "c".to_string(),
        CallOverride {
            result_type: Some("char*".to_string()),
            ..CallOverride::default()
        },
    );

    let result_type_name = resolve_call_info(&call, "c", CallIr::default(), None).result_type_name;

    assert_eq!(
        result_type_name
            .require()
            .expect("an explicit override always resolves"),
        "char*"
    );
    assert!(
        logs_contain("disables nested-field verification"),
        "a primitive/pointer result_type override must warn that it disables verification"
    );
}

/// Negative control: a genuine PascalCase override is exactly what the `result_type`
/// field's own doc comment (and `unresolved_result_type_name`'s "set `result_type` on the
/// call override" advice) recommend when the IR cannot model a call at all. That legitimate
/// use must stay silent.
#[tracing_test::traced_test]
#[test]
fn resolve_call_info_stays_silent_for_a_genuine_pascal_case_result_type_override() {
    use crate::e2e::config::CallOverride;

    let mut call = call_named("legacy_export");
    call.overrides.insert(
        "c".to_string(),
        CallOverride {
            result_type: Some("LegacyExportResult".to_string()),
            ..CallOverride::default()
        },
    );

    let result_type_name = resolve_call_info(&call, "c", CallIr::default(), None).result_type_name;

    assert_eq!(
        result_type_name
            .require()
            .expect("an explicit override always resolves"),
        "LegacyExportResult"
    );
    assert!(
        !logs_contain("disables nested-field verification"),
        "a real PascalCase type name plugging an IR gap is the documented, intended use and \
         must not warn"
    );
}
