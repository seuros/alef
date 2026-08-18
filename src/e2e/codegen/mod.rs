//! E2e test code generation trait and language dispatch.
//!
//! ## DRY layer ([`client`])
//!
//! Per-language e2e codegen historically duplicated the structural shape of every
//! test (function header, request build, response assert) and only differed in
//! syntax. The [`client`] submodule pulls that shape into trait + driver pairs
//! ([`client::TestClientRenderer`] + [`client::http_call::render_http_test`])
//! so each language can be migrated to TestClient-driven tests by:
//!
//! 1. Implementing `TestClientRenderer` once per language (small, mechanical).
//! 2. Replacing the language's monolithic `render_http_test_function` with a
//!    call to `client::http_call::render_http_test(out, &MyRenderer, fixture)`.
//! 3. Optionally splitting the per-language file into a directory
//!    `<lang>/{mod.rs,client.rs,ws.rs,helpers.rs}` when the file gets unwieldy.
//!
//! Until a language migrates, it continues using the legacy monolithic renderer —
//! both can coexist behind the per-language [`E2eCodegen::generate`] entry.

pub mod assertion_recipes;
pub(crate) mod assertion_type_skip;
pub mod assertion_types;
pub mod brew;
pub mod c;
pub mod client;
pub mod client_factory;
pub mod csharp;
pub mod dart;
mod dart_visitors;
pub mod elixir;
pub(crate) mod field_skip;
pub mod gleam;
pub mod go;
pub mod homebrew;
pub mod java;
mod java_mvnw;
pub mod kotlin;
pub mod kotlin_android;
pub mod php;
pub mod php_ext;
mod presentation;
pub mod python;
pub mod r;
pub mod recipe;
pub mod ruby;
pub mod rust;
pub mod streaming_assertions;
pub mod swift;
mod swift_visitors;
pub mod typescript;
pub mod wasm;
pub mod zig;
mod zig_visitors;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, MethodDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;

/// Check if a fixture should be included for the given language.
///
/// Returns false if:
/// - The fixture's resolved category is in `e2e_config.exclude_categories`
///   (fixture is excluded from every language's cross-language e2e codegen)
/// - The fixture has a skip condition that applies to this language
/// - The fixture's call has no resolvable function for this language (no base
///   `function` set and no override for the language). Calls that share a base
///   function but only carry per-language type/arg overrides are still emitted
///   for languages without an explicit override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InclusionDecision {
    Include,
    Exclude(&'static str),
}

impl InclusionDecision {
    pub fn is_included(&self) -> bool {
        matches!(self, Self::Include)
    }
}

pub fn fixture_inclusion(fixture: &Fixture, language: &str, e2e_config: &E2eConfig) -> InclusionDecision {
    if !e2e_config.exclude_categories.is_empty() && e2e_config.exclude_categories.contains(&fixture.resolved_category())
    {
        return InclusionDecision::Exclude("excluded category");
    }
    if let Some(skip) = &fixture.skip
        && skip.should_skip(language)
    {
        return InclusionDecision::Exclude("fixture skip directive");
    }
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Also respect skip_languages on the resolved call (e.g. batch_scrape skips elixir).
    if call_config.skip_languages.iter().any(|l| l == language) {
        return InclusionDecision::Exclude("call skips language");
    }
    // HTTP/mock fixtures are exercised by issuing a request to the alef mock server
    // (`MOCK_SERVER_URL/fixtures/<id>`), not by invoking a binding function, so they are
    // includable even when no call `function` is resolved for the language. Function-call
    // consumers (fixtures without `mock_response`/`http`) still require a resolved function
    // or a per-language override, leaving their behaviour unchanged.
    let is_http_fixture = fixture.mock_response.is_some() || fixture.http.is_some();
    if !is_http_fixture && call_config.function.is_empty() && !call_config.overrides.contains_key(language) {
        return InclusionDecision::Exclude("no callable function");
    }
    InclusionDecision::Include
}

pub(crate) fn should_include_fixture(fixture: &Fixture, language: &str, e2e_config: &E2eConfig) -> bool {
    fixture_inclusion(fixture, language, e2e_config).is_included()
}

/// Percent-encode a string for use as a URI query component per RFC 3986.
///
/// Only the unreserved set (`ALPHA / DIGIT / "-" / "." / "_" / "~"`) is left
/// literal; every other byte (spaces, `?`, `&`, `=`, non-ASCII, …) is `%XX`-escaped.
/// Used by per-language e2e generators that embed query parameters into a request URL
/// literal — without this, values like `hi there` produce an invalid URI and the
/// generated test throws at parse time instead of exercising the fixture.
pub(crate) fn percent_encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Recursively rewrite a JSON value's object keys to the target wire case.
///
/// `wire_case` accepts the same vocabulary as serde's `rename_all` attribute:
/// `"snake_case"` (default), `"camelCase"`, `"PascalCase"`, `"SCREAMING_SNAKE_CASE"`,
/// `"kebab-case"`, `"SCREAMING-KEBAB-CASE"`. Unknown values fall back to `snake_case`.
///
/// Used by per-language e2e codegen to translate canonical (snake_case) fixture keys
/// to the wire case that each binding's `from_json` / typed deserializer expects, as
/// driven by `ResolvedCrateConfig::serde_rename_all_for_language`.
pub(crate) fn transform_json_keys_for_language(value: &serde_json::Value, wire_case: &str) -> serde_json::Value {
    use heck::{ToKebabCase, ToLowerCamelCase, ToPascalCase, ToShoutyKebabCase, ToShoutySnakeCase, ToSnakeCase};
    let rewrite_key: fn(&str) -> String = match wire_case {
        "camelCase" => |k| k.to_lower_camel_case(),
        "PascalCase" => |k| k.to_pascal_case(),
        "SCREAMING_SNAKE_CASE" => |k| k.to_shouty_snake_case(),
        "kebab-case" => |k| k.to_kebab_case(),
        "SCREAMING-KEBAB-CASE" => |k| k.to_shouty_kebab_case(),
        _ => |k| k.to_snake_case(),
    };
    fn walk(value: &serde_json::Value, rewrite_key: fn(&str) -> String) -> serde_json::Value {
        match value {
            serde_json::Value::Object(obj) => {
                let new_obj: serde_json::Map<String, serde_json::Value> = obj
                    .iter()
                    .map(|(k, v)| (rewrite_key(k), walk(v, rewrite_key)))
                    .collect();
                serde_json::Value::Object(new_obj)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| walk(v, rewrite_key)).collect())
            }
            other => other.clone(),
        }
    }
    walk(value, rewrite_key)
}

/// Placeholder that e2e fixtures can embed inside structured JSON arguments.
///
/// This is useful for APIs where a URL lives inside a request DTO rather than in a
/// top-level `mock_url` argument. Language generators replace the token at test
/// runtime with the per-fixture mock server base URL.
pub(crate) const MOCK_URL_PLACEHOLDER: &str = "$mock_url";

/// Return true when a fixture value recursively contains [`MOCK_URL_PLACEHOLDER`].
pub(crate) fn value_contains_mock_url_placeholder(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => value.contains(MOCK_URL_PLACEHOLDER),
        serde_json::Value::Array(values) => values.iter().any(value_contains_mock_url_placeholder),
        serde_json::Value::Object(values) => values.values().any(value_contains_mock_url_placeholder),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => false,
    }
}

/// Environment variable used by the mock server for fixtures with a host-root listener.
pub(crate) fn mock_url_env_key(fixture_id: &str) -> String {
    format!("MOCK_SERVER_{}", fixture_id.to_uppercase())
}

/// The error text an `error` assertion declares, if any.
///
/// ~keep Backends must match this against the rendered message **or** the exception/
/// variant name, never message-only. Fixture authors use both conventions: config
/// validation fixtures name a field that appears in user-facing message text and never
/// in a type name, while API-error fixtures name a type prefix such as `Authentication`
/// that never appears in the message. The disjunction is what lets one codegen path
/// serve both, and narrowing it silently breaks whichever convention it drops.
pub(crate) fn declared_error_value(fixture: &crate::e2e::fixture::Fixture) -> Option<&str> {
    fixture
        .assertions
        .iter()
        .find(|assertion| assertion.assertion_type == "error")
        .and_then(|assertion| assertion.value.as_ref())
        .and_then(serde_json::Value::as_str)
}

/// The literal URL a `mock_url` argument must be given verbatim, if any.
///
/// ~keep `preserve` is the fixture's `preserve_input_urls` flag and `value` is the
/// already-resolved `input.<field>` the emitter holds — every `mock_url` branch
/// receives that value today and discards it in favour of the mock server address.
/// Backends must consult this *before* binding the mock server and fall through
/// unchanged on `None`, because substituting the mock address silently rewrites the
/// subject of any test whose point is the address itself.
pub(crate) fn preserved_url_literal(preserve: bool, value: &serde_json::Value) -> Option<&str> {
    if !preserve {
        return None;
    }
    value.as_str()
}

/// The literal URL list a `mock_url_list` argument must be given verbatim, if any.
///
/// ~keep The list counterpart of [`preserved_url_literal`]. `value` is whatever the
/// backend already resolved (via [`resolve_urls_field`], so `batch_urls` ↔ `urls`
/// aliasing still applies). A list containing any non-string entry yields `None`
/// rather than a partially-preserved list: dropping an element silently would weaken
/// the test in precisely the way the substitution being replaced does.
pub(crate) fn preserved_url_list(preserve: bool, value: &serde_json::Value) -> Option<Vec<&str>> {
    if !preserve {
        return None;
    }
    value.as_array()?.iter().map(serde_json::Value::as_str).collect()
}

/// Environment variable that arms the loud-failure path for e2e assertions whose
/// field the availability oracle (`FieldResolver::is_valid_for_result`) rejects.
///
/// ~keep Every backend's `render_assertion` downgrades a rejected field to a
/// `<comment-open> skipped: <`[`field_skip::FieldSkip`]`>` comment and returns —
/// the generated test still compiles and still passes, because nothing asserted
/// anything. Each backend keeps its own reason prose; recognition is shared, so
/// registering a new wording as a `FieldSkip` variant is what arms it here and no
/// backend can emit a skip this gate cannot count. That is the defect this
/// module addresses: unset (or any value other
/// than `"1"`/`"true"`), [`fail_on_unavailable_field_markers`] is a pure no-op and
/// every backend's generated output is byte-identical to before this file changed.
/// Set, it turns the same skip comment into a generation-time panic naming the
/// fixture and field, matching how this codebase already fails loudly elsewhere at
/// generation time (`TestBackendEmission`'s removed `unimplemented()` constructor,
/// the Python synthetic-field panics in `python/assertions.rs`).
///
/// Two backends do not have IR-derived field data threaded into their
/// `FieldResolver` (`gleam`, `brew` — see their `assertions.rs`/`test_case.rs`
/// construction sites): for those, arming this still consults only the
/// hand-maintained `result_fields` TOML list, which is known to drift in both
/// directions. Their generators still call [`fail_on_unavailable_field_markers`]
/// (the mechanism itself does not special-case a backend), but the oracle behind
/// `is_valid_for_result` is coarser there, so arming this globally will very
/// likely fire *more* false positives on gleam/brew fixtures than on the other 14
/// backends until they get the same IR wiring. `homebrew` never constructs a
/// `FieldResolver` at all (it generates a Brewfile + shell smoke script, not
/// fixture-driven assertions) and cannot hit this path.
pub(crate) const STRICT_ASSERTIONS_ENV: &str = "ALEF_E2E_STRICT_ASSERTIONS";

/// ~keep The default is now ON, which is a deliberate reversal. The previous default was OFF and
/// that is precisely how the debt accumulated: an opt-in gate that nobody opted into is
/// indistinguishable from no gate at all, and a survey of the committed e2e trees found 177
/// rendered skip markers across four consumer repos — including whole expected-event-sequence
/// assertions that were inert in every language that emitted them. Adding a *second* opt-in flag
/// would have repeated a control that had already failed once. Setting this variable to `0` or
/// `false` (or passing `--no-strict-assertions`) restores the old lenient behaviour for an
/// emergency regeneration; the end-of-run summary is printed either way, so turning it off
/// downgrades the failure to a visible number rather than to silence.
fn strict_assertions_default() -> bool {
    true
}

/// True unless [`STRICT_ASSERTIONS_ENV`] is explicitly set to a disarming value.
///
/// Reads the process environment directly; call sites that need a unit-testable (env-independent)
/// core should exercise [`strict_assertion_failure`] with an explicit `strict` bool instead of
/// mutating process env in a test — mutating shared process env from parallel `#[test]` runs is
/// not independent.
pub(crate) fn strict_assertions_enabled() -> bool {
    std::env::var(STRICT_ASSERTIONS_ENV)
        .ok()
        .map_or_else(strict_assertions_default, |raw| !is_falsy_flag(&raw))
}

fn is_falsy_flag(raw: &str) -> bool {
    raw == "0" || raw.eq_ignore_ascii_case("false")
}

/// ~keep `gleam` and `brew` do not have IR-derived field data threaded into their `FieldResolver`
/// (see their `assertions.rs` / `test_case.rs` construction sites): their oracle consults only the
/// hand-maintained `result_fields` TOML list, which is known to drift in both directions. A
/// rejection there is not trustworthy enough to fail a build on, so their authoring gaps are
/// recorded and summarised but never fatal. Remove a backend from this list once its resolver is
/// IR-wired. `homebrew` never constructs a `FieldResolver` at all and cannot reach this path.
const COARSE_FIELD_ORACLE_LANGUAGES: &[&str] = &["gleam", "brew"];

/// What the gate decided about one rendered skip marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipVerdict {
    /// A fixable resolution failure the fixture did not acknowledge. Fatal when strict.
    UnacknowledgedGap,
    /// A real language/ABI limit, or a gap from a backend whose oracle is too coarse to trust.
    Limitation,
    /// alef cannot express this assertion shape yet. Never fatal — the debt is alef's, not the
    /// consumer's, and no fixture edit clears it.
    AwaitingGeneratorSupport,
    /// The fixture explicitly declared this assertion skipped for this language, and named which
    /// backlog it belongs to.
    Acknowledged(crate::e2e::fixture::AssertionSkipKind),
}

/// Which axis a [`SkipRecord`] was recognised on.
///
/// ~keep [`field_skip::FieldSkip`] answers "does this field exist on the result?";
/// [`assertion_type_skip::AssertionTypeSkip`] answers "can this backend express this assertion
/// *shape* at all?". They are recorded onto the same ledger (so [`skip_summary`] reports one
/// number and [`SkipVerdict`]'s three-way class still applies to both), but kept distinguishable
/// here rather than merged into one axis — merging would either force every
/// `AssertionTypeSkip` variant to accept a `FieldSkip`-shaped acknowledgement path it structurally
/// cannot use (a bad assertion shape is never a fixture's mistake to acknowledge) or blur the
/// per-assertion-type attribution the type axis exists to provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipOrigin {
    /// Recognised by [`field_skip::FieldSkip`]: a field path the availability oracle rejected.
    Field,
    /// Recognised by [`assertion_type_skip::AssertionTypeSkip`]: an assertion type (or, for the
    /// one wording that never named it, a field) a backend's renderer cannot express.
    AssertionType,
}

/// One rendered skip marker, with the fixture and language it came from.
#[derive(Debug, Clone)]
pub(crate) struct SkipRecord {
    pub(crate) language: String,
    pub(crate) fixture_id: String,
    /// The token the marker's wording captured: a field path for [`SkipOrigin::Field`], an
    /// assertion type (or, for the one wording that never named it, a field) for
    /// [`SkipOrigin::AssertionType`].
    pub(crate) field: String,
    pub(crate) verdict: SkipVerdict,
    pub(crate) origin: SkipOrigin,
}

thread_local! {
    /// ~keep Thread-local rather than a `Mutex` global: e2e generation runs the backends
    /// sequentially on the driver's thread, and a thread-local keeps `#[test]` cases independent
    /// for free (cargo gives each test its own thread), which a process-global ledger would not.
    static SKIP_LEDGER: std::cell::RefCell<Vec<SkipRecord>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Drain every skip recorded on this thread since the last drain.
pub(crate) fn take_skip_records() -> Vec<SkipRecord> {
    SKIP_LEDGER.with(|ledger| std::mem::take(&mut *ledger.borrow_mut()))
}

/// The one-line residual-debt summary, or `None` when nothing was skipped.
///
/// ~keep Printed on every run, strict or not: a skip that is legitimate today is still an
/// assertion that is not running, and this count is the only thing that makes that visible.
/// The buckets are deliberately separate rather than one total, because they have different
/// owners — `awaiting alef support` is a queue of generator features, `language/ABI limit` is
/// usually permanent, and `unresolved field path` is the only one a consumer can fix today.
/// Collapsing them would make the number un-actionable and it would stop being read.
pub(crate) fn skip_summary(records: &[SkipRecord]) -> Option<String> {
    if records.is_empty() {
        return None;
    }
    use crate::e2e::fixture::AssertionSkipKind;
    let fixtures: std::collections::BTreeSet<&str> = records.iter().map(|r| r.fixture_id.as_str()).collect();
    let count = |predicate: fn(&SkipVerdict) -> bool| records.iter().filter(|r| predicate(&r.verdict)).count();
    let awaiting = count(|v| {
        matches!(
            v,
            SkipVerdict::AwaitingGeneratorSupport | SkipVerdict::Acknowledged(AssertionSkipKind::NotRepresentable)
        )
    });
    let limitations = count(|v| {
        matches!(
            v,
            SkipVerdict::Limitation | SkipVerdict::Acknowledged(AssertionSkipKind::LanguageLimitation)
        )
    });
    let gaps = count(|v| matches!(v, SkipVerdict::UnacknowledgedGap));
    // ~keep Appended only when at least one record came from the assertion-type axis, so every
    // existing summary produced from `fail_on_unavailable_field_markers` alone (the only source
    // before this axis existed) renders byte-identical to before.
    let type_skips = records.iter().filter(|r| r.origin == SkipOrigin::AssertionType).count();
    let type_skip_suffix = if type_skips > 0 {
        format!(", {type_skips} from an assertion type this backend cannot render rather than an unavailable field")
    } else {
        String::new()
    };
    Some(format!(
        "{} assertion(s) skipped across {} fixture(s): {awaiting} awaiting alef support, \
         {limitations} language/ABI limitation(s), {gaps} unresolved field path(s){type_skip_suffix}",
        records.len(),
        fixtures.len(),
    ))
}

/// Scan every line of a fully-rendered assertions body for a registered skip marker, classify each
/// against the fixture's own assertions, and record it on the ledger.
///
/// ~keep This records but never fails. Enforcement lives in the driver
/// ([`strict_assertion_failure`]) for two reasons: `run_generators` isolates a backend's `Err` but
/// does not catch a panic, so failing here would abort the whole run instead of one backend; and a
/// consumer facing many unresolved paths needs to see all of them in one error, not to fix one and
/// rediscover the next on the following run.
///
/// Called once per fixture, after every assertion has been rendered into `body` — the same point
/// every backend's existing vacuous-assertion-body fallback (where one exists, e.g. python's
/// `apply_vacuous_assertion_fallback`) already inspects the finished text, so wiring this in adds
/// no new call-site shape to any backend.
pub(crate) fn fail_on_unavailable_field_markers(
    body: &str,
    language: &str,
    fixture_id: &str,
    assertions: &[crate::e2e::fixture::Assertion],
) {
    let coarse_oracle = COARSE_FIELD_ORACLE_LANGUAGES.contains(&language);
    for line in body.lines() {
        let Some((field, skip)) = field_skip::FieldSkip::extract_classified(line) else {
            continue;
        };
        let declared = assertions
            .iter()
            .filter(|assertion| assertion.field.as_deref() == Some(field))
            .find_map(|assertion| assertion.skip.as_ref())
            .filter(|skip| skip.should_skip(language));
        let verdict = match (declared, skip.class()) {
            (Some(declaration), _) => SkipVerdict::Acknowledged(declaration.kind()),
            (None, field_skip::SkipClass::GeneratorGap) => SkipVerdict::AwaitingGeneratorSupport,
            (None, field_skip::SkipClass::AuthoringGap) if !coarse_oracle => SkipVerdict::UnacknowledgedGap,
            (None, _) => SkipVerdict::Limitation,
        };
        SKIP_LEDGER.with(|ledger| {
            ledger.borrow_mut().push(SkipRecord {
                language: language.to_string(),
                fixture_id: fixture_id.to_string(),
                field: field.to_string(),
                verdict,
                origin: SkipOrigin::Field,
            });
        });
    }
}

/// Scan every line of a fully-rendered assertions body for an [`assertion_type_skip::AssertionTypeSkip`]
/// marker and record it on the ledger.
///
/// ~keep The field-axis counterpart, [`fail_on_unavailable_field_markers`], cross-references the
/// fixture's own assertions to decide `Acknowledged` vs. `UnacknowledgedGap` — that acknowledgement
/// path exists because an unresolved field CAN be a fixable authoring mistake. An assertion-type
/// skip never is: [`assertion_type_skip::AssertionTypeSkip::class`] never returns
/// [`field_skip::SkipClass::AuthoringGap`], so there is no gap for a fixture to acknowledge and
/// this function does not take an `assertions` slice. `SkipClass::AuthoringGap` is still handled
/// below (mapped defensively to `Limitation`) so the match stays exhaustive if that invariant is
/// ever violated by a future variant, without panicking mid-generation.
pub(crate) fn fail_on_unsupported_assertion_type_markers(body: &str, language: &str, fixture_id: &str) {
    for line in body.lines() {
        let Some((token, skip)) = assertion_type_skip::AssertionTypeSkip::extract_classified(line) else {
            continue;
        };
        let verdict = match skip.class() {
            field_skip::SkipClass::GeneratorGap => SkipVerdict::AwaitingGeneratorSupport,
            field_skip::SkipClass::LanguageLimitation | field_skip::SkipClass::AuthoringGap => SkipVerdict::Limitation,
        };
        SKIP_LEDGER.with(|ledger| {
            ledger.borrow_mut().push(SkipRecord {
                language: language.to_string(),
                fixture_id: fixture_id.to_string(),
                field: token.to_string(),
                verdict,
                origin: SkipOrigin::AssertionType,
            });
        });
    }
}

/// Env-independent core of the loud-failure path: the error for every unacknowledged authoring gap
/// recorded this run, or `None` when there are none or `strict` is off.
///
/// Every offender is listed, deduplicated by (fixture, field, language) ordering, so one
/// regeneration surfaces the whole authoring backlog rather than its first entry. ~keep
pub(crate) fn strict_assertion_failure(records: &[SkipRecord], strict: bool) -> Option<anyhow::Error> {
    if !strict {
        return None;
    }
    let gaps: Vec<&SkipRecord> = records
        .iter()
        .filter(|r| r.verdict == SkipVerdict::UnacknowledgedGap)
        .collect();
    if gaps.is_empty() {
        return None;
    }
    let detail = gaps
        .iter()
        .map(|r| format!("  [{}] fixture `{}`: field `{}`", r.language, r.fixture_id, r.field))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n");
    Some(anyhow::anyhow!(
        "{} e2e assertion(s) reference a field the availability oracle cannot resolve, so they \
         would have been silently dropped and the generated tests would have passed while \
         asserting nothing:\n{detail}\n\nEither fix the field path (or the field-availability \
         config) so the assertion runs, or declare on the assertion why it cannot:\n  \
         \"skip\": {{ \"kind\": \"not_representable\", \"reason\": \"...\" }}      — alef cannot \
         express this shape yet (an assertion *kind* such as \"the call errored\", a property of \
         the call rather than the result, or an assertion over a stream's events)\n  \
         \"skip\": {{ \"kind\": \"language_limitation\", \"languages\": [\"<lang>\"], \
         \"reason\": \"...\" }}  — this binding genuinely cannot reach the field\nEither way the \
         skip stays counted in the end-of-run summary, in the bucket that names who owns it. Set \
         {}=0 to downgrade this to a warning for one run.",
        gaps.len(),
        STRICT_ASSERTIONS_ENV,
    ))
}

#[cfg(test)]
mod unavailable_field_marker_tests {
    use super::{
        SkipVerdict, fail_on_unavailable_field_markers, is_falsy_flag, skip_summary, strict_assertion_failure,
        take_skip_records,
    };
    use crate::e2e::fixture::{Assertion, AssertionSkip, AssertionSkipDirective, AssertionSkipKind};

    /// Record `body` and return the verdicts, so a wording's *classification* can be asserted
    /// directly rather than inferred from whether generation failed. ~keep
    fn verdicts_for(body: &str, language: &str, assertions: &[Assertion]) -> Vec<SkipVerdict> {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(body, language, "smoke", assertions);
        take_skip_records().into_iter().map(|r| r.verdict).collect()
    }

    /// The strict-mode error for one rendered body, or `None` if it is generatable.
    fn strict_error_for(body: &str, language: &str, fixture_id: &str, assertions: &[Assertion]) -> Option<String> {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(body, language, fixture_id, assertions);
        strict_assertion_failure(&take_skip_records(), true).map(|error| format!("{error:#}"))
    }

    fn assertion_on(field: &str, skip: Option<AssertionSkip>) -> Assertion {
        Assertion {
            field: Some(field.to_string()),
            skip,
            ..Assertion::default()
        }
    }

    /// The escape hatch must still generate: with strict off, a marker body is not an error.
    #[test]
    fn non_strict_is_a_noop_even_on_a_marker_body() {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(
            "    # skipped: field 'chunks' not available on result type\n",
            "python",
            "widget_smoke",
            &[],
        );
        assert!(strict_assertion_failure(&take_skip_records(), false).is_none());
    }

    #[test]
    fn strict_fails_loudly_naming_fixture_and_field() {
        let error = strict_error_for(
            "    # skipped: field 'chunks' not available on result type\n",
            "python",
            "widget_smoke",
            &[],
        )
        .expect("an unresolved field must fail under strict");
        assert!(
            error.contains("[python] fixture `widget_smoke`: field `chunks`"),
            "got: {error}"
        );
    }

    /// The headline requirement: an unmappable field must fail generation *by default*, with no
    /// environment variable armed. This reads the real default rather than a hard-coded `true`,
    /// so flipping the default back would fail this test. ~keep
    #[test]
    fn an_unmappable_field_is_fatal_by_default() {
        let _ = take_skip_records();
        let body = "    // skipped: field 'strategy.crawl_order' not available on result type\n";
        fail_on_unavailable_field_markers(
            body,
            "go",
            "traversal_order",
            &[assertion_on("strategy.crawl_order", None)],
        );
        let error = strict_assertion_failure(&take_skip_records(), super::strict_assertions_enabled())
            .expect("an unmappable field must fail generation by default");
        assert!(
            format!("{error:#}").contains("field `strategy.crawl_order`"),
            "got: {error:#}"
        );
    }

    /// Every offender is listed, not just the first — one regeneration must surface the whole
    /// backlog. ~keep
    #[test]
    fn strict_error_lists_every_offender() {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(
            "    // skipped: field 'alpha' not available on result type\n\
             \x20   // skipped: field 'beta' not available on result type\n",
            "go",
            "smoke",
            &[],
        );
        let error = strict_assertion_failure(&take_skip_records(), true).expect("two gaps must fail");
        let rendered = format!("{error:#}");
        assert!(rendered.contains("field `alpha`"), "got: {rendered}");
        assert!(rendered.contains("field `beta`"), "got: {rendered}");
        assert!(rendered.starts_with("2 e2e assertion(s)"), "got: {rendered}");
    }

    /// The opt-in half: the same body, with the fixture declaring the skip, generates cleanly and
    /// is recorded as an acknowledged skip rather than silently vanishing. ~keep
    #[test]
    fn an_explicitly_opted_out_field_skips_and_is_counted() {
        let _ = take_skip_records();
        let body = "    // skipped: field 'strategy.crawl_order' not available on result type\n";
        let assertions = [assertion_on(
            "strategy.crawl_order",
            Some(AssertionSkip::Scoped(AssertionSkipDirective {
                languages: vec!["go".to_string()],
                kind: AssertionSkipKind::LanguageLimitation,
                reason: Some("traversal order is not exposed on the Go result".to_string()),
            })),
        )];
        fail_on_unavailable_field_markers(body, "go", "traversal_order", &assertions);

        let records = take_skip_records();
        assert!(
            strict_assertion_failure(&records, true).is_none(),
            "an acknowledged skip must not fail generation"
        );
        assert_eq!(records.len(), 1, "the acknowledged skip must still be recorded");
        assert_eq!(
            records[0].verdict,
            SkipVerdict::Acknowledged(AssertionSkipKind::LanguageLimitation)
        );
        assert_eq!(records[0].field, "strategy.crawl_order");
        let summary = skip_summary(&records).expect("an acknowledged skip must still produce a summary");
        assert_eq!(
            summary,
            "1 assertion(s) skipped across 1 fixture(s): 0 awaiting alef support, \
             1 language/ABI limitation(s), 0 unresolved field path(s)"
        );
    }

    /// An opt-out declared `not_representable` lands in alef's backlog, not the binding's, so the
    /// summary attributes it to alef rather than filing it as a consumer limitation. ~keep
    #[test]
    fn a_not_representable_opt_out_is_attributed_to_alef() {
        let _ = take_skip_records();
        let body = "    // skipped: field 'is_error' not available on result type\n";
        let assertions = [assertion_on(
            "is_error",
            Some(AssertionSkip::Scoped(AssertionSkipDirective {
                languages: Vec::new(),
                kind: AssertionSkipKind::NotRepresentable,
                reason: Some("`is_error` is an assertion kind, not a field path".to_string()),
            })),
        )];
        fail_on_unavailable_field_markers(body, "go", "error_smoke", &assertions);
        let records = take_skip_records();
        assert!(strict_assertion_failure(&records, true).is_none());
        let summary = skip_summary(&records).expect("summary");
        assert!(summary.contains("1 awaiting alef support"), "got: {summary}");
        assert!(summary.contains("0 language/ABI limitation(s)"), "got: {summary}");
    }

    /// A `"skip": true` opt-out covers every language, and defaults to alef's backlog — the
    /// observed common case is an assertion shape alef cannot express, not a binding limit.
    #[test]
    fn a_bare_true_skip_covers_every_language() {
        let body = "    // skipped: field 'chunks' not available on result type\n";
        let assertions = [assertion_on("chunks", Some(AssertionSkip::All(true)))];
        let expected = [SkipVerdict::Acknowledged(AssertionSkipKind::NotRepresentable)];
        assert_eq!(verdicts_for(body, "dart", &assertions), expected);
        assert_eq!(verdicts_for(body, "ruby", &assertions), expected);
    }

    /// A language-scoped opt-out must not silence the same field in a language it does not name.
    #[test]
    fn a_scoped_skip_does_not_cover_other_languages() {
        let body = "    // skipped: field 'chunks' not available on result type\n";
        let assertions = [assertion_on(
            "chunks",
            Some(AssertionSkip::Scoped(AssertionSkipDirective {
                languages: vec!["dart".to_string()],
                kind: AssertionSkipKind::LanguageLimitation,
                reason: None,
            })),
        )];
        assert_eq!(
            verdicts_for(body, "dart", &assertions),
            vec![SkipVerdict::Acknowledged(AssertionSkipKind::LanguageLimitation)]
        );
        assert_eq!(
            verdicts_for(body, "go", &assertions),
            vec![SkipVerdict::UnacknowledgedGap],
            "an opt-out scoped to dart must leave go fatal"
        );
    }

    /// `"skip": false` is an explicit *refusal* to opt out and must behave as if absent.
    #[test]
    fn a_false_skip_does_not_opt_out() {
        let body = "    // skipped: field 'chunks' not available on result type\n";
        let assertions = [assertion_on("chunks", Some(AssertionSkip::All(false)))];
        assert_eq!(
            verdicts_for(body, "go", &assertions),
            vec![SkipVerdict::UnacknowledgedGap]
        );
    }

    /// An opt-out on a *different* field must not silence this one.
    #[test]
    fn a_skip_on_another_field_does_not_opt_this_one_out() {
        let body = "    // skipped: field 'chunks' not available on result type\n";
        let assertions = [assertion_on("usage", Some(AssertionSkip::All(true)))];
        assert_eq!(
            verdicts_for(body, "go", &assertions),
            vec![SkipVerdict::UnacknowledgedGap]
        );
    }

    #[test]
    fn a_body_with_no_marker_records_nothing() {
        assert!(verdicts_for("    assert result.count == 1  # noqa: S101\n", "python", &[]).is_empty());
    }

    /// Regression control: a synthetic field's "unsupported assertion type" comment
    /// is a different defect (bad assertion shape) and must not trip this check.
    #[test]
    fn unsupported_assertion_type_comments_are_not_recorded() {
        assert!(
            verdicts_for(
                "    // skipped: unsupported assertion type on synthetic field 'embeddings'\n",
                "go",
                &[]
            )
            .is_empty()
        );
    }

    /// The language-suffixed variants (`not available on Python ProcessingResult`,
    /// `not available on PHP result type`, ...) resolve against a *generated* binding type, so
    /// they are gaps and stay fatal.
    #[test]
    fn language_suffixed_not_available_comments_stay_fatal() {
        let error = strict_error_for(
            "\t// skipped: field 'keywords' not available on Go ProcessingResult\n",
            "go",
            "smoke",
            &[],
        )
        .expect("a binding-type resolution miss is a gap");
        assert!(error.contains("field `keywords`"), "got: {error}");
    }

    /// ~keep This is the variant that hid whole expected-event-sequence assertions, so it is
    /// tempting to make it fatal. It must not be: a streaming call returns an event sequence, not
    /// a struct, so no field mapping can express the assertion and a consumer cannot fix it from
    /// their own config. Failing their build would force a blanket opt-out — the silent skip
    /// again, with ceremony. It is loudly *counted* against alef's backlog instead.
    #[test]
    fn streaming_field_assertions_await_alef_support_rather_than_failing() {
        let body = "    // streaming assertion on unsupported field 'has_page_event'\n";
        assert_eq!(
            verdicts_for(body, "csharp", &[]),
            vec![SkipVerdict::AwaitingGeneratorSupport]
        );
        assert!(
            strict_error_for(body, "csharp", "stream_smoke", &[]).is_none(),
            "a missing generator feature must not fail a consumer's build"
        );
    }

    /// The same holds for python's streaming-accessor wording and the streaming result type.
    #[test]
    fn every_streaming_wording_awaits_alef_support() {
        for (body, language) in [
            (
                "    # skipped: streaming field 'stream.items': no python accessor\n",
                "python",
            ),
            (
                "    // skipped: field 'stream.items' not available on streaming result type\n",
                "go",
            ),
        ] {
            assert_eq!(
                verdicts_for(body, language, &[]),
                vec![SkipVerdict::AwaitingGeneratorSupport],
                "{language} streaming wording must be alef's debt, not the consumer's"
            );
        }
    }

    /// The summary must keep alef's backlog and the binding's backlog in separate buckets, or the
    /// number stops being actionable and stops being read. ~keep
    #[test]
    fn summary_separates_alef_backlog_from_binding_limits() {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(
            "    // streaming assertion on unsupported field 'has_page_event'\n",
            "csharp",
            "stream_smoke",
            &[],
        );
        fail_on_unavailable_field_markers(
            "        // skipped: field 'usage.tokens' references a field or type excluded from \
             the Swift binding\n",
            "swift",
            "excluded_smoke",
            &[],
        );
        let summary = skip_summary(&take_skip_records()).expect("summary");
        assert_eq!(
            summary,
            "2 assertion(s) skipped across 2 fixture(s): 1 awaiting alef support, \
             1 language/ABI limitation(s), 0 unresolved field path(s)"
        );
    }

    /// ~keep The wordings below are all still *recognised* — that is the invariant the shared
    /// `FieldSkip` table exists to hold — but they name real language/ABI limits, so recognition
    /// now means "counted in the summary", not "fails the build". Asserting the verdict rather
    /// than a panic is what keeps that distinction honest: if one of them were ever reclassified
    /// as a gap, these assertions fail rather than silently changing the default's blast radius.
    #[test]
    fn tagged_union_boundary_wordings_are_counted_not_fatal() {
        let dart = "    // skipped: field 'payload.tags' crosses a tagged-union variant boundary \
                    (not expressible in Dart)\n";
        assert_eq!(verdicts_for(dart, "dart", &[]), vec![SkipVerdict::Limitation]);
        let swift = "    // skipped: field 'payload.tags' crosses a tagged-union variant boundary \
                     (not expressible in Swift)\n";
        assert_eq!(verdicts_for(swift, "swift", &[]), vec![SkipVerdict::Limitation]);
    }

    #[test]
    fn ruby_serialized_enum_accessor_wording_is_counted_not_fatal() {
        let body = "    # skipped: enum variant accessor 'metadata.format.excel' not available on Ruby \
                    (serialized to Hash)\n";
        assert_eq!(verdicts_for(body, "ruby", &[]), vec![SkipVerdict::Limitation]);
    }

    #[test]
    fn result_is_simple_template_wording_is_counted_not_fatal() {
        let body = "        // skipped: result_is_simple, field 'metadata.title' not on simple result type\n";
        assert_eq!(verdicts_for(body, "php", &[]), vec![SkipVerdict::Limitation]);
    }

    #[test]
    fn not_applicable_for_simple_result_wording_is_counted_not_fatal() {
        let body = "    # skipped: field 'structure.headings' not applicable for simple result type\n";
        assert_eq!(verdicts_for(body, "python", &[]), vec![SkipVerdict::Limitation]);
    }

    #[test]
    fn swift_binding_exclusion_wording_is_counted_not_fatal() {
        let body = "        // skipped: field 'usage.tokens' references a field or type excluded from \
                    the Swift binding\n";
        assert_eq!(verdicts_for(body, "swift", &[]), vec![SkipVerdict::Limitation]);
    }

    /// `result_is_simple for field '<x>' not available on result type` is the *resolver* talking
    /// despite the prefix, so it stays a gap while the other `result_is_simple` wordings do not.
    #[test]
    fn the_result_is_simple_resolver_wording_stays_a_gap() {
        let body = "  # skipped: result_is_simple for field 'metadata' not available on result type\n";
        assert_eq!(verdicts_for(body, "ruby", &[]), vec![SkipVerdict::UnacknowledgedGap]);
    }

    /// gleam and brew resolve fields against a hand-maintained TOML list rather than the IR, so a
    /// rejection there is not trustworthy enough to fail a build on. ~keep
    #[test]
    fn coarse_oracle_backends_downgrade_gaps_to_limitations() {
        let body = "    // skipped: field 'chunks' not available on result type\n";
        assert_eq!(verdicts_for(body, "gleam", &[]), vec![SkipVerdict::Limitation]);
        assert_eq!(verdicts_for(body, "brew", &[]), vec![SkipVerdict::Limitation]);
        assert_eq!(
            verdicts_for(body, "go", &[]),
            vec![SkipVerdict::UnacknowledgedGap],
            "the same wording must stay fatal on an IR-wired backend"
        );
    }

    #[test]
    fn summary_is_none_when_nothing_was_skipped() {
        assert_eq!(skip_summary(&[]), None);
    }

    #[test]
    fn summary_counts_distinct_fixtures_not_markers() {
        let _ = take_skip_records();
        let body = "    // skipped: field 'usage.tokens' references a field or type excluded from \
                    the Swift binding\n";
        fail_on_unavailable_field_markers(body, "swift", "alpha", &[]);
        fail_on_unavailable_field_markers(body, "swift", "alpha", &[]);
        fail_on_unavailable_field_markers(body, "swift", "beta", &[]);
        let summary = skip_summary(&take_skip_records()).expect("three markers must summarise");
        assert!(
            summary.starts_with("3 assertion(s) skipped across 2 fixture(s):"),
            "got: {summary}"
        );
    }

    #[test]
    fn is_falsy_flag_accepts_zero_and_case_insensitive_false_only() {
        assert!(is_falsy_flag("0"));
        assert!(is_falsy_flag("false"));
        assert!(is_falsy_flag("FALSE"));
        assert!(!is_falsy_flag("1"));
        assert!(!is_falsy_flag("true"));
        assert!(!is_falsy_flag(""));
        assert!(!is_falsy_flag("no"));
    }

    /// The diagnostic has to be actionable on its own: it must name where to look and how to
    /// declare the skip, or a consumer hitting it has no path forward but to disable the gate.
    #[test]
    fn diagnostic_names_language_fixture_field_and_the_opt_in() {
        let message = strict_error_for(
            "    # skipped: field 'usage' not available on result type\n",
            "ruby",
            "batch_smoke",
            &[],
        )
        .expect("an unresolved field must fail under strict");
        assert!(message.contains("[ruby]"), "got: {message}");
        assert!(message.contains("`batch_smoke`"), "got: {message}");
        assert!(message.contains("`usage`"), "got: {message}");
        assert!(message.contains("\"skip\""), "must name the opt-in: {message}");
        assert!(
            message.contains(super::STRICT_ASSERTIONS_ENV),
            "must name the escape hatch: {message}"
        );
    }
}

#[cfg(test)]
mod assertion_type_marker_tests {
    use super::{SkipOrigin, SkipVerdict, fail_on_unsupported_assertion_type_markers, skip_summary, take_skip_records};

    fn verdicts_for(body: &str, language: &str) -> Vec<SkipVerdict> {
        let _ = take_skip_records();
        fail_on_unsupported_assertion_type_markers(body, language, "smoke");
        take_skip_records().into_iter().map(|r| r.verdict).collect()
    }

    /// Every record this gate produces must be tagged [`SkipOrigin::AssertionType`], never
    /// [`SkipOrigin::Field`] — the two axes must stay distinguishable downstream.
    #[test]
    fn recorded_markers_are_tagged_with_the_assertion_type_origin() {
        let _ = take_skip_records();
        fail_on_unsupported_assertion_type_markers(
            "\t// skipped: unsupported assertion type on synthetic field 'embeddings'\n",
            "go",
            "smoke",
        );
        let records = take_skip_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].origin, SkipOrigin::AssertionType);
    }

    /// GeneratorGap-classified wordings (alef's own debt) never fail a build, mirroring the field
    /// axis's `AwaitingGeneratorSupport` treatment.
    #[test]
    fn generator_gap_wordings_await_alef_support() {
        let body = "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'\n";
        assert_eq!(verdicts_for(body, "go"), vec![SkipVerdict::AwaitingGeneratorSupport]);
    }

    /// LanguageLimitation-classified wordings are counted as a real limitation, not alef's debt.
    #[test]
    fn language_limitation_wordings_are_counted_as_limitations() {
        let body = "        // skipped: field 'content' is a scalar String without meaningful .count\n";
        assert_eq!(verdicts_for(body, "swift"), vec![SkipVerdict::Limitation]);
    }

    /// Regression control: an ordinary field-availability skip must not be picked up by this gate
    /// — the two funnels (`FieldSkip` / `AssertionTypeSkip`) stay disjoint.
    #[test]
    fn field_availability_markers_are_not_recorded_by_this_gate() {
        let body = "    // skipped: field 'chunks' not available on result type\n";
        assert!(verdicts_for(body, "python").is_empty());
    }

    #[test]
    fn a_body_with_no_marker_records_nothing() {
        assert!(verdicts_for("    assert result.count == 1\n", "python").is_empty());
    }

    /// The one-line summary must call out how many of the skips came from the assertion-type axis
    /// rather than the field axis, or the two are indistinguishable in the number a consumer reads.
    #[test]
    fn summary_calls_out_assertion_type_skips_separately() {
        let _ = take_skip_records();
        fail_on_unsupported_assertion_type_markers(
            "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'\n",
            "go",
            "traversal_smoke",
        );
        let summary = skip_summary(&take_skip_records()).expect("summary");
        assert!(
            summary.contains("1 from an assertion type this backend cannot render"),
            "got: {summary}"
        );
    }
}

/// Trait for per-language e2e test code generation.
pub trait E2eCodegen: Send + Sync {
    /// Generate all e2e test project files for this language.
    ///
    /// `type_defs` is the IR type registry extracted from the source crate.
    /// It is used by backends that need to introspect struct field types at
    /// codegen time (e.g. the TypeScript/WASM generator uses it to
    /// auto-derive `nested_types` mappings for wasm-bindgen class wrapping).
    ///
    /// `enums` is the IR enum registry extracted from the source crate.
    /// For WASM, it is used to identify tagged-data enums so they are emitted
    /// as plain JS object literals instead of wrapper factories.
    ///
    /// `functions` is the IR free-function registry (`ApiSurface::functions`) —
    /// free `pub fn`s only; inherent and trait methods live on
    /// [`TypeDef::methods`] and are reachable through `type_defs`. Backends use
    /// it to derive a call's result type from the declared return type instead
    /// of inventing one from the call name; a name invented that way is not a
    /// real type, and every IR-keyed check downstream of it (the C backend's
    /// nested-field verification, for one) default-allows rather than fails, so
    /// a wrong name here silently disables verification instead of breaking
    /// generation. ~keep
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
    ) -> Result<Vec<GeneratedFile>>;

    /// The assertion `type` values this backend can render.
    ///
    /// Defaults to the full schema-known set minus this language's row in
    /// [`assertion_types::BACKEND_UNSUPPORTED_ASSERTION_TYPES`], so a backend added later
    /// is covered by [`Self::generate_gated`] without touching its own file. ~keep
    fn supported_assertion_types(&self) -> Vec<&'static str> {
        assertion_types::supported_assertion_types(self.language_name())
    }

    /// Run the shared fixture gates, then this backend's [`Self::generate`].
    ///
    /// Every driver must call this rather than `generate` directly: it is the one place a
    /// cross-backend gate can be added without a per-backend edit, and the one place that
    /// turns an unrenderable assertion into an error naming the fixture it came from
    /// instead of an empty render, a stray comment, or a panic. ~keep
    fn generate_gated(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
    ) -> Result<Vec<GeneratedFile>> {
        assertion_types::ensure_supported_assertion_types(
            groups,
            e2e_config,
            self.language_name(),
            &self.supported_assertion_types(),
        )?;
        self.generate(groups, e2e_config, config, type_defs, enums, functions)
    }

    /// Render the target-language source inside a generated documentation snippet.
    fn render_snippet_body(
        &self,
        _fixture: &Fixture,
        _e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
        _type_defs: &[TypeDef],
        _enums: &[EnumDef],
    ) -> Result<String> {
        anyhow::bail!(
            "e2e code generator `{}` does not support documentation snippets",
            self.language_name()
        )
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[TypeDef],
        enums: &[EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
    ) -> Result<String> {
        self.render_snippet_body(fixture, e2e_config, config, type_defs, enums)
    }

    /// Language name for display and directory naming.
    fn language_name(&self) -> &'static str;
}

/// Get all available e2e code generators.
pub fn all_generators() -> Vec<Box<dyn E2eCodegen>> {
    vec![
        Box::new(rust::RustE2eCodegen),
        Box::new(python::PythonE2eCodegen),
        Box::new(typescript::TypeScriptCodegen),
        Box::new(go::GoCodegen),
        Box::new(java::JavaCodegen),
        Box::new(kotlin::KotlinE2eCodegen),
        Box::new(kotlin_android::KotlinAndroidE2eCodegen),
        Box::new(csharp::CSharpCodegen),
        Box::new(php::PhpCodegen),
        Box::new(php_ext::PhpExtCodegen),
        Box::new(ruby::RubyCodegen),
        Box::new(elixir::ElixirCodegen),
        Box::new(gleam::GleamE2eCodegen),
        Box::new(r::RCodegen),
        Box::new(wasm::WasmCodegen),
        Box::new(c::CCodegen),
        Box::new(zig::ZigE2eCodegen),
        Box::new(dart::DartE2eCodegen),
        Box::new(swift::SwiftE2eCodegen),
        Box::new(brew::BrewCodegen),
        Box::new(homebrew::HomebrewCodegen),
    ]
}

/// Get e2e code generators for specific language names.
pub fn generators_for(languages: &[String]) -> Vec<Box<dyn E2eCodegen>> {
    all_generators()
        .into_iter()
        .filter(|g| languages.iter().any(|l| l == g.language_name()))
        .collect()
}

/// Resolve a JSON field from a fixture input by path.
///
/// Field paths in call config are "input.path", "input.config", etc.
/// Since we already receive `fixture.input`, strip the leading "input." prefix.
/// When `field_path` is exactly `"input"`, the whole input object is returned.
pub(crate) fn resolve_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // "input" with no subpath means "the entire input object".
    if field_path == "input" {
        // New fixture schema wraps the call input DTO under `extract_input`
        // alongside a sibling `mock_responses` array (so a single fixture can both
        // declare the input and configure the mock server). Unwrap it so the arg
        // resolves to the actual DTO. Flat fixtures — where `input` *is* the DTO —
        // have no `extract_input` key and are returned unchanged.
        if let Some(inner) = input.get("extract_input") {
            return inner;
        }
        return input;
    }
    let path = field_path.strip_prefix("input.").unwrap_or(field_path);
    let mut current = input;
    for part in path.split('.') {
        current = current.get(part).unwrap_or(&serde_json::Value::Null);
    }
    current
}

/// Select the best-matching call for a fixture based on input field availability.
///
/// When the initially resolved call config has required args whose fields are
/// missing from fixture input, search the named calls for one whose args better
/// match the available input fields. This allows generic call selection even when
/// select_when conditions are too specific (e.g., category-restricted).
///
/// Returns the passed-in `initial_call` if no better match is found.
pub(crate) fn select_best_matching_call<'a>(
    initial_call: &'a crate::e2e::config::CallConfig,
    e2e_config: &'a E2eConfig,
    fixture: &Fixture,
) -> &'a crate::e2e::config::CallConfig {
    // Check if initial call's required args can be satisfied from fixture input
    let initial_satisfied = initial_call.args.iter().all(|arg| {
        if arg.optional {
            return true;
        }
        // For mock_url_list args, use resolve_urls_field which handles aliasing
        // (e.g., batch_urls ↔ urls). For other arg types, use regular resolve_field.
        let field_value = if arg.arg_type == "mock_url_list" {
            resolve_urls_field(&fixture.input, &arg.field)
        } else {
            resolve_field(&fixture.input, &arg.field)
        };
        field_value.as_null().is_none()
    });

    if initial_satisfied {
        return initial_call;
    }

    // Initial call has unsatisfied required args. Search named calls for a better match.
    for alt_call in e2e_config.calls.values() {
        let all_satisfied = alt_call.args.iter().all(|arg| {
            if arg.optional {
                return true;
            }
            // For mock_url_list args, use resolve_urls_field which handles aliasing
            // (e.g., batch_urls ↔ urls). For other arg types, use regular resolve_field.
            let field_value = if arg.arg_type == "mock_url_list" {
                resolve_urls_field(&fixture.input, &arg.field)
            } else {
                resolve_field(&fixture.input, &arg.field)
            };
            field_value.as_null().is_none()
        });

        if all_satisfied {
            return alt_call;
        }
    }

    // No better call found; use initial
    initial_call
}

/// Resolve a list-type argument field, trying both the declared field name and
/// common aliases (batch_urls, urls; urls_list, url_list).
///
/// Used by codegen for `mock_url_list` arguments when the fixture may use
/// alternative field names (e.g. some fixtures use `urls` while call config
/// declares `batch_urls`).
pub(crate) fn resolve_urls_field<'a>(input: &'a serde_json::Value, field_path: &str) -> &'a serde_json::Value {
    // Try the declared field first
    let result = resolve_field(input, field_path);
    if !result.is_null() {
        return result;
    }

    // Try common aliases if the primary field is not found
    let aliases = [
        ("batch_urls", "urls"),
        ("urls", "batch_urls"),
        ("batch_urls", "url_list"),
        ("batch_urls", "urls_list"),
        ("urls", "url_list"),
        ("urls", "urls_list"),
    ];

    for (orig, alias) in &aliases {
        if field_path.ends_with(orig) {
            let aliased_path = field_path.replace(orig, alias);
            let result = resolve_field(input, &aliased_path);
            if !result.is_null() {
                return result;
            }
        }
    }

    // Nothing found; return null
    &serde_json::Value::Null
}

/// Emission result for a test backend stub.
///
/// There is deliberately no "nothing emitted" constructor and no `Default` impl:
/// every `TestBackendEmission` a caller obtains must carry a real, compilable stub.
/// A language or configuration that cannot produce one panics before a value is
/// ever returned (see [`emit_test_backend`] and each per-language emitter) rather
/// than handing back a placeholder callers must remember to check. ~keep
#[derive(Debug, Clone)]
pub struct TestBackendEmission {
    /// Code emitted at the top of the test function: stub class/struct definition.
    pub setup_block: String,
    /// Expression passed as the register_X arg: stub instance or Bridge-wrapped instance.
    pub arg_expr: String,
    /// Short symbol names that must be imported at the file or function scope
    /// for the generated stub to compile.  Rust backend populates this with
    /// the trait name and any named return/parameter types so that callers can
    /// emit the appropriate `use module::Symbol;` statements.  Other language
    /// backends leave this empty — they manage imports internally.
    pub type_imports: Vec<String>,
    /// Optional teardown statements emitted after the fixture call and its
    /// assertions, used to undo registry mutations performed by trait-bridge
    /// fixtures (e.g. `unregister_ocr_backend("test-backend")`).
    ///
    /// Test runners that share a process across tests (python pytest, ruby
    /// rspec, dart `test`, etc.) leak registered test backends into later
    /// tests; without a teardown the next OCR-using fixture fails because the
    /// global registry contains only `test-backend` and the core's
    /// `ensure_ocr_backends_initialized` self-heal only triggers when the
    /// registry is empty. Emitting `unregister_<trait>(<name>)` here drains
    /// the test backend so the next access re-seeds the defaults.
    ///
    /// Languages that run each test in its own process (Rust cargo
    /// integration tests, Go) leave this empty.
    pub teardown_block: String,
}

/// Dispatch test backend emission to per-language implementations.
///
/// When a fixture argument has `arg_type = "test_backend"`, this dispatcher
/// resolves the trait bridge config and calls the language-specific emitter.
/// Backends that haven't implemented test backend emission yet panic rather
/// than return a placeholder — see [`TestBackendEmission`]'s doc comment.
pub fn emit_test_backend(
    language: &str,
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&MethodDef],
    fixture: &Fixture,
    enums: &[crate::core::ir::EnumDef],
) -> TestBackendEmission {
    match language {
        "rust" => rust::emit_test_backend(trait_bridge, methods, fixture),
        "python" => python::emit_test_backend(trait_bridge, methods, fixture),
        "typescript" | "wasm" => typescript::emit_test_backend(trait_bridge, methods, fixture, enums),
        "node" => typescript::emit_test_backend(trait_bridge, methods, fixture, enums), // node uses typescript codegen
        "go" => go::emit_test_backend(trait_bridge, methods, fixture),
        "java" => java::emit_test_backend(trait_bridge, methods, fixture, ""),
        "kotlin" => kotlin::emit_test_backend(trait_bridge, methods, fixture),
        "kotlin_android" => kotlin_android::emit_test_backend(trait_bridge, methods, fixture),
        "csharp" => csharp::emit_test_backend(trait_bridge, methods, fixture),
        "php" => php::emit_test_backend(trait_bridge, methods, fixture),
        "ruby" => ruby::emit_test_backend(trait_bridge, methods, fixture),
        "elixir" => elixir::emit_test_backend(trait_bridge, methods, fixture, "", ""),
        "gleam" => gleam::emit_test_backend(trait_bridge, methods, fixture),
        "r" => r::emit_test_backend(trait_bridge, methods, fixture),
        "c" => c::emit_test_backend(trait_bridge, methods, fixture),
        "zig" => zig::emit_test_backend(trait_bridge, methods, fixture),
        "dart" => dart::emit_test_backend(trait_bridge, methods, fixture, &[]),
        "swift" => swift::emit_test_backend(trait_bridge, methods, fixture, &[]),
        "brew" => brew::emit_test_backend(trait_bridge, methods, fixture),
        "php_ext" => php_ext::emit_test_backend(trait_bridge, methods, fixture),
        "homebrew" => homebrew::emit_test_backend(trait_bridge, methods, fixture),
        _ => panic!(
            "e2e codegen: no test_backend emitter registered for language `{language}`; \
             cannot generate a test_backend stub for this target"
        ),
    }
}

#[cfg(test)]
mod preserved_url_tests {
    use super::{preserved_url_list, preserved_url_literal};

    #[test]
    fn scalar_url_is_preserved_only_when_requested() {
        let value = serde_json::json!("http://127.0.0.1/private");
        assert_eq!(preserved_url_literal(true, &value), Some("http://127.0.0.1/private"));
        assert_eq!(preserved_url_literal(false, &value), None);
    }

    #[test]
    fn url_list_is_preserved_atomically() {
        let value = serde_json::json!(["http://host-a.test/", "file:///tmp/example"]);
        assert_eq!(
            preserved_url_list(true, &value),
            Some(vec!["http://host-a.test/", "file:///tmp/example"])
        );
        assert_eq!(
            preserved_url_list(true, &serde_json::json!(["https://host.test", 7])),
            None
        );
    }
}

#[cfg(test)]
mod unimplemented_test_backend_tests {
    use super::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::e2e::fixture::Fixture;

    fn registered_bridge() -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "SampleBackend".into(),
            ..TraitBridgeConfig::default()
        }
    }

    fn sample_fixture() -> Fixture {
        Fixture {
            id: "register_sample_backend".into(),
            ..Fixture::default()
        }
    }

    /// Every language with no real `test_backend` stub generator (including a
    /// language string the dispatch `match` doesn't even recognize) must panic
    /// through the public dispatcher rather than hand back a placeholder
    /// `TestBackendEmission`. There is no sentinel value left to construct — the
    /// `unimplemented()` constructor and `UNIMPLEMENTED_MARKER` were removed —
    /// so this is the structural proof that a caller can never receive a
    /// stand-in emission for these targets, whether or not the trait is
    /// registered: a language with no real generator fails the same way
    /// regardless of `trait_bridges` config.
    #[test]
    fn languages_without_a_real_emitter_panic_through_the_dispatcher() {
        for language in ["gleam", "brew", "php_ext", "homebrew", "kotlin", "not-a-real-language"] {
            let bridge = registered_bridge();
            let fixture = sample_fixture();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                emit_test_backend(language, &bridge, &[], &fixture, &[])
            }));
            assert!(
                result.is_err(),
                "expected `{language}` to panic instead of returning a TestBackendEmission, but it returned a value"
            );
        }
    }
}
