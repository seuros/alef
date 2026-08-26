use super::*;

fn test_backend_arg(trait_name: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "backend".into(),
        field: "backend".into(),
        arg_type: "test_backend".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: Some(trait_name.to_string()),
    }
}

/// Pin: a `test_backend` arg whose trait IS registered still panics today,
/// because `c::emit_test_backend` (`trait_bridge_snippet.rs`) is unimplemented —
/// see its doc comment for why. `emit_test_backend` panics before ever handing
/// `build_args_string_c` a value, so there is no sentinel left to accidentally
/// splice into the call's argument list. This is the regression guard: it fails
/// if that panic is ever replaced with a placeholder return and the call site
/// stops checking it.
#[test]
#[should_panic(expected = "test-backend emitter is unimplemented")]
fn registered_test_backend_trait_panics_because_c_backend_is_unimplemented() {
    use crate::core::config::TraitBridgeConfig;

    let bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".into(),
        ..TraitBridgeConfig::default()
    };
    let config = ResolvedCrateConfig {
        trait_bridges: vec![bridge],
        ..ResolvedCrateConfig::default()
    };
    let fixture = Fixture {
        id: "register_sample_backend".into(),
        ..Fixture::default()
    };
    let args = vec![test_backend_arg("SampleBackend")];

    let _ = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "register_sample_backend",
        TargetParams::IrAbsent,
    );
}

/// An unregistered trait (no matching `[[crates.trait_bridges]]` entry) has no
/// vtable to point at — generation must fail loudly instead of falling back to
/// `NULL`. Unlike Kotlin's non-null interface parameter, nothing in C's type
/// system would catch a bad `NULL` default at compile time, so this loud check
/// is the only thing standing between a misconfigured `alef.toml` and either an
/// uncompilable comment or a `NULL` vtable pointer reaching generated C.
#[test]
#[should_panic(expected = "no `[[crates.trait_bridges]]` entry")]
fn unregistered_test_backend_trait_panics_instead_of_falling_back_to_null() {
    let config = ResolvedCrateConfig::default();
    let fixture = Fixture {
        id: "register_sample_backend".into(),
        ..Fixture::default()
    };
    let args = vec![test_backend_arg("SampleBackend")];

    let _ = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "register_sample_backend",
        TargetParams::IrAbsent,
    );
}

/// Regression for the bug that shipped a `char[37]` literal against a
/// `TS_PACKAlefHandle` (an `int32_t`) parameter: with no `args` configured, alef
/// used to splice the fixture's whole `input` JSON as a single C string literal
/// regardless of the target's real parameters, which cannot compile against
/// anything the target actually takes. A genuinely zero-argument target
/// (`TargetParams::Known(&[])`) is the one case that must keep emitting an empty
/// argument list rather than refuse. ~keep
#[test]
fn should_emit_empty_parens_when_args_unconfigured_and_target_takes_no_parameters() {
    let fixture = Fixture {
        id: "list_ocr_backends".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();

    let result = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "list_ocr_backends",
        TargetParams::Known(&[]),
    )
    .expect("a genuinely zero-argument target must not fail generation");

    assert_eq!(
        result, "",
        "a zero-argument call must emit `()`, not a fabricated literal"
    );
}

/// The actual defect this guards: `ts_pack_configure` takes one typed parameter
/// (`config`, an opaque handle), but the fixture configured no `args`. Splicing the
/// whole fixture `input` JSON as one C string literal produced
/// `ts_pack_configure("{\"cache_dir\":...}")` against `int32_t
/// ts_pack_configure(TS_PACKAlefHandle config)` -- an incompatible
/// pointer-to-integer conversion that does not compile. The emitter must refuse
/// with a diagnostic instead of guessing an argument it cannot construct. ~keep
#[test]
fn should_refuse_when_args_unconfigured_and_target_takes_a_typed_parameter() {
    let fixture = Fixture {
        id: "pack_configure_defaults".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let params = [ParamDef {
        name: "config".into(),
        ..ParamDef::default()
    }];

    let error = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "ts_pack_configure",
        TargetParams::Known(&params),
    )
    .expect_err("a known non-empty parameter list must not be papered over with a JSON literal")
    .to_string();

    assert!(
        !error.contains("cache_dir"),
        "must not leak the fixture JSON into a diagnostic that replaces splicing it: {error}"
    );
    assert!(error.contains("ts_pack_configure"), "must name the call: {error}");
    assert!(error.contains("config"), "must name the unfilled parameter: {error}");
    assert!(error.contains("args"), "must point at the `args` config knob: {error}");
}

/// When the IR signature cannot be resolved at all, the emitter has no basis to
/// tell a genuine zero-argument call from an authoring gap -- refuse rather than
/// guess, per the same principle `ResultTypeName::require` applies to result types.
#[test]
fn should_refuse_when_args_unconfigured_and_target_signature_is_unresolvable() {
    let fixture = Fixture {
        id: "mystery_call".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();

    let error = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "mystery_fn",
        TargetParams::Unresolvable,
    )
    .expect_err("an unresolvable signature must not fall back to guessing")
    .to_string();

    assert!(error.contains("mystery_fn"), "must name the call: {error}");
    assert!(error.contains("args"), "must point at the `args` config knob: {error}");
}

/// The boundary between the two refusing cases and the one that must not refuse.
///
/// `IrAbsent` means no IR was consulted at all -- the main e2e test-file emitter has no
/// `CallIr`, and several snippet entry points render without one. Nothing was learned, so
/// nothing can be concluded, and this keeps the pre-existing behaviour instead of failing.
/// Collapsing it back into `Unresolvable` would fail generation for every IR-less caller,
/// which is a far wider blast radius than the defect this guards, and it would put this
/// half of the fix in direct contradiction with `unresolved_result_type_name`, which
/// classifies an absent IR as `Unverified` for exactly the same reason. Both halves must
/// agree on what an absent IR licenses, or one of them is wrong. ~keep
#[test]
fn should_keep_prior_behaviour_when_there_is_no_ir_to_consult() {
    let fixture = Fixture {
        id: "no_ir".into(),
        input: serde_json::json!({"cache_dir": "/tmp/sample_cache"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();

    let rendered = build_args_string_c(
        &fixture.input,
        &[],
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "sample_fn",
        TargetParams::IrAbsent,
    )
    .expect("an absent IR must not fail generation on a path that never had a signature");

    assert_eq!(
        rendered,
        json_to_c(&fixture.input),
        "with no IR consulted the emitter must render exactly what it rendered before"
    );
}

/// The load-bearing control: a call WITH properly configured `args` must keep
/// emitting them, unchanged, real typed literal and all. Without this test, a fix
/// that makes the empty-`args` path refuse (or always emit `()`) everywhere would
/// pass the two tests above and look correct while quietly breaking every snippet
/// that already configures `args` correctly -- the two failure modes above only
/// ever trigger on `args.is_empty()`, so nothing else in this suite would catch a
/// regression that clobbers the non-empty path too. ~keep
#[test]
fn should_still_emit_configured_args_unchanged_when_args_are_present() {
    let fixture = Fixture {
        id: "chat_basic".into(),
        input: serde_json::json!({"text": "hello"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![crate::e2e::config::ArgMapping {
        name: "text".into(),
        field: "text".into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];

    // `TargetParams::Unresolvable` on purpose: an unresolved signature licenses no claim
    // about any parameter's type, so a configured `args` list must render exactly as it
    // always did. (A resolved signature does license one -- see
    // `should_refuse_a_string_literal_configured_against_a_handle_parameter` and its
    // correctly-typed control below.)
    let result = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "chat",
        TargetParams::Unresolvable,
    )
    .expect("configured args must still render");

    assert_eq!(
        result, "\"hello\"",
        "a configured string arg must still emit its real typed literal"
    );
}

fn string_arg(name: &str, field: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: name.into(),
        field: field.into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// The other half of the same defect. The refusals above all key on `args.is_empty()` --
/// "no args configured, do not fabricate an argument list". This is the opposite case:
/// `args` are present, so the arity is satisfied and nothing refuses, but the entry's type
/// contradicts the parameter's. `json_to_c` stringifies the JSON object and the emitter
/// splices a `char[]` literal into a parameter the C ABI exports as `AlefHandle` -- the
/// same `-Wint-conversion` failure, reached without ever passing through the empty-`args`
/// guard. ~keep
#[test]
fn should_refuse_a_string_literal_configured_against_a_handle_parameter() {
    let fixture = Fixture {
        id: "configure_cache_dir".into(),
        input: serde_json::json!({"config": {"cache_dir": "/tmp/sample_cache"}}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![string_arg("config", "config")];
    let params = [ParamDef {
        name: "config".into(),
        ty: TypeRef::Named("SampleConfig".into()),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "SampleConfig".into(),
        has_serde: true,
        ..TypeDef::default()
    }];

    let error = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &type_defs,
        &fixture,
        "sample_configure",
        TargetParams::Known(&params),
    )
    .expect_err("a JSON object must not be lowered into a handle parameter")
    .to_string();

    assert!(error.contains("sample_configure"), "must name the call: {error}");
    assert!(error.contains("`config`"), "must name the parameter: {error}");
    assert!(
        error.contains("AlefHandle"),
        "must name the parameter's C type: {error}"
    );
    assert!(
        error.contains("cache_dir"),
        "must quote the offending value so the operator can find the entry: {error}"
    );
    assert!(
        error.contains("json_object"),
        "must name the configuration that constructs the handle: {error}"
    );
}

/// The false-refusal boundary, and the reason this check cannot simply reject every JSON
/// object. A `Vec<Named>` parameter does NOT cross the C ABI as a handle -- `type_map`'s
/// `c_param_type` maps it to `*const c_char`, a JSON string -- so the stringified literal
/// is exactly the right lowering there. Refusing it would delete correct, compiling
/// documentation, which is why `handle_param_type_name` deliberately does not unwrap
/// through `Vec` the way `c.rs`'s `named_type` does. ~keep
#[test]
fn should_not_refuse_a_json_literal_against_a_vec_parameter() {
    let fixture = Fixture {
        id: "rank_documents".into(),
        input: serde_json::json!({"documents": ["alpha", "beta"]}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![string_arg("documents", "documents")];
    let params = [ParamDef {
        name: "documents".into(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".into()))),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "Document".into(),
        has_serde: true,
        ..TypeDef::default()
    }];

    let rendered = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &type_defs,
        &fixture,
        "sample_rank",
        TargetParams::Known(&params),
    )
    .expect("a JSON-string parameter must keep rendering its literal");

    assert_eq!(
        rendered,
        json_to_c(&fixture.input["documents"]),
        "a `Vec<T>` parameter crosses as a JSON `const char *`, so the literal is correct"
    );
}

/// A parameter type the IR names but carries no `TypeDef` for cannot be proven to be a
/// handle: an IR enum is an `EnumDef`, never a `TypeDef`, and enum-typed `Named` parameters
/// cross as `i32`. Refusing on the name alone would reject every enum argument on evidence
/// the emitter does not have, so an unmatched name leaves the rendering untouched. ~keep
#[test]
fn should_not_refuse_a_named_parameter_the_ir_carries_no_type_def_for() {
    let fixture = Fixture {
        id: "set_level".into(),
        input: serde_json::json!({"level": "debug"}),
        ..Fixture::default()
    };
    let config = ResolvedCrateConfig::default();
    let args = vec![string_arg("level", "level")];
    let params = [ParamDef {
        name: "level".into(),
        ty: TypeRef::Named("LogLevel".into()),
        ..ParamDef::default()
    }];

    let rendered = build_args_string_c(
        &fixture.input,
        &args,
        &HashMap::new(),
        &config,
        &[],
        &fixture,
        "sample_set_level",
        TargetParams::Known(&params),
    )
    .expect("a name with no `TypeDef` behind it licenses no claim about the C type");

    assert_eq!(rendered, "\"debug\"", "the rendering must be left exactly as it was");
}
