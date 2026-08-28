use super::*;

/// Every test drains the thread-local ledger it uses; cargo gives each test its own thread, so
/// no test can observe another's entries.
fn drained() -> String {
    take_error("node").map(|error| format!("{error:#}")).unwrap_or_default()
}

#[test]
fn an_empty_ledger_produces_no_error() {
    assert!(
        take_error("node").is_none(),
        "nothing was recorded, so nothing may fail"
    );
}

/// The regression this whole module exists for: recording a refusal must NOT unwind. Before it,
/// the same condition was a `panic!` five frames below the backend's `generate`, which aborted
/// the entire `alef all` process at exit 101 and skipped every later stage. ~keep
#[test]
fn recording_a_refusal_does_not_panic() {
    record("PackConfig", "cache_dir", RefusalSite::Argument);
    assert_eq!(take().len(), 1, "the refusal must be on the ledger, not on the stack");
}

/// The message must name the call, the language, and -- decisively -- that the type was
/// INHERITED from the file-level default rather than chosen for this call, because the fix
/// belongs at the per-call level and editing the file-level default re-types every other call.
#[test]
fn an_inherited_file_level_options_type_names_the_per_call_table_as_the_fix() {
    record("ParseOptions", "cache_dir", RefusalSite::Argument);
    attribute("node", "pack-basic", Some("pack"), OptionsTypeSource::LanguageDefault);

    let message = drained();

    assert!(
        message.contains("fixture `pack-basic`"),
        "must name the fixture: {message}"
    );
    assert!(message.contains("`[e2e.calls.pack]`"), "must name the call: {message}");
    assert!(message.contains("language `node`"), "must name the language: {message}");
    assert!(
        message.contains("`[e2e.call.overrides.node].options_type` default"),
        "must name the level the wrong type came from: {message}"
    );
    assert!(
        message.contains("declare `options_type` under `[e2e.calls.pack.overrides.node]`"),
        "must name the level the fix belongs at: {message}"
    );
    assert!(
        message.contains("applies to every call that does not override it"),
        "must say why editing the file-level default is the wrong fix: {message}"
    );
}

/// A per-call `options_type` is a type the author chose for exactly this call, so the message
/// must not send them to the file-level default -- all three remedies are equally live.
#[test]
fn a_per_call_options_type_offers_the_three_remedies_without_the_inheritance_warning() {
    record("ParseOptions", "cache_dir", RefusalSite::Argument);
    attribute("node", "pack-basic", Some("pack"), OptionsTypeSource::PerCall);

    let message = drained();

    assert!(
        message.contains("`[e2e.calls.pack.overrides.node]`.options_type"),
        "must name where the type came from: {message}"
    );
    assert!(
        message.contains("names the wrong type for this call"),
        "the override itself must be offered as the cause: {message}"
    );
    assert!(
        !message.contains("applies to every call that does not override it"),
        "nothing was inherited here, so the inheritance warning must not appear: {message}"
    );
}

/// With no `options_type` at any level the override is still the lever worth naming, but the
/// message must not claim a level supplied the type.
#[test]
fn an_unset_options_type_still_names_the_override_as_the_lever() {
    record("PackInput", "cache_dir", RefusalSite::Argument);
    attribute("wasm", "pack-basic", None, OptionsTypeSource::Unset);

    let message = drained();

    assert!(
        message.contains("No `options_type` is configured for `wasm`"),
        "must state that nothing pinned the type: {message}"
    );
    assert!(
        message.contains("`[e2e.call.overrides.wasm]`"),
        "the default call's own override table is the lever: {message}"
    );
    assert!(
        message.contains("`[e2e.call]` (the default call)"),
        "an unnamed call must still be identified: {message}"
    );
}

/// The old text told the operator to "fix the fixture ... or the Rust struct". In the incident
/// that produced this module both were already correct, and the message cost an experienced
/// reader about an hour. Naming the wrong remedy is worse than naming none. ~keep
#[test]
fn an_inherited_default_does_not_present_the_fixture_or_struct_as_the_only_remedies() {
    record("ParseOptions", "cache_dir", RefusalSite::Argument);
    attribute("node", "pack-basic", Some("pack"), OptionsTypeSource::LanguageDefault);

    let message = drained();
    let options_type_lever = message
        .find("declare `options_type` under")
        .expect("lever must be present");
    let fixture_remedy = message
        .find("remove or rename the fixture key")
        .expect("remedy present");

    assert!(
        options_type_lever < fixture_remedy,
        "the config lever must be offered before the fixture/struct remedies: {message}"
    );
}

#[test]
fn a_nested_refusal_reports_the_path_it_was_reached_through() {
    record(
        "RetryPolicy",
        "max_attempts",
        RefusalSite::Nested {
            via: "field `retry` of `PackConfig`".to_string(),
        },
    );
    attribute("node", "pack-basic", Some("pack"), OptionsTypeSource::PerCall);

    let message = drained();

    assert!(
        message.contains("reached through field `retry` of `PackConfig`"),
        "a nested refusal must say where it sat: {message}"
    );
}

/// Every refusal must survive the drain, not just the first: one bad `options_type` typically
/// refuses several keys at once, and reporting one per regeneration is a serial debugging loop.
#[test]
fn every_recorded_refusal_reaches_the_error() {
    record("ParseOptions", "cache_dir", RefusalSite::Argument);
    record("ParseOptions", "worker_count", RefusalSite::Argument);
    attribute("node", "pack-basic", Some("pack"), OptionsTypeSource::LanguageDefault);

    let message = drained();

    assert!(message.contains("refused 2 fixture value(s)"), "got: {message}");
    assert!(message.contains("`cache_dir`"), "got: {message}");
    assert!(message.contains("`worker_count`"), "got: {message}");
}

#[test]
fn attribution_does_not_overwrite_an_earlier_fixtures_context() {
    record("ParseOptions", "cache_dir", RefusalSite::Argument);
    attribute(
        "node",
        "first-fixture",
        Some("pack"),
        OptionsTypeSource::LanguageDefault,
    );
    record("ParseOptions", "worker_count", RefusalSite::Argument);
    attribute("node", "second-fixture", Some("parse"), OptionsTypeSource::PerCall);

    let message = drained();

    assert!(message.contains("fixture `first-fixture`"), "got: {message}");
    assert!(message.contains("fixture `second-fixture`"), "got: {message}");
}

#[test]
fn taking_the_error_clears_the_ledger() {
    record("ParseOptions", "cache_dir", RefusalSite::Argument);
    assert!(take_error("node").is_some());
    assert!(
        take_error("node").is_none(),
        "a drained refusal must not be reported again by the next backend"
    );
}

/// `resolved_call_key` must answer by identity. Two named calls can be field-for-field equal
/// while only one of them is the config this fixture actually routed to; a value comparison
/// would name the wrong `[e2e.calls.<key>]` table in the diagnostic. ~keep
#[test]
fn the_call_key_is_resolved_by_identity_not_by_equality() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    let call = crate::core::config::e2e::CallConfig {
        function: "pack".to_string(),
        ..Default::default()
    };
    e2e_config.calls.insert("pack".to_string(), call.clone());
    e2e_config.calls.insert("pack_twin".to_string(), call);

    let pack = &e2e_config.calls["pack"];
    let twin = &e2e_config.calls["pack_twin"];

    assert_eq!(resolved_call_key(&e2e_config, pack), Some("pack"));
    assert_eq!(resolved_call_key(&e2e_config, twin), Some("pack_twin"));
    assert_eq!(
        resolved_call_key(&e2e_config, &e2e_config.call),
        None,
        "the default call has no `[e2e.calls.<key>]` name"
    );
}
