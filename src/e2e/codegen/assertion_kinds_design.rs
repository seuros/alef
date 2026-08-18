//! SPECIFICATION SKETCH — not wired into `mod.rs`, nothing references this module.
//!
//! ~keep This file is the type-level half of `docs/design/assertion-kinds.md`. It exists so an
//! implementer reads signatures rather than re-deriving them from prose. Every item here is
//! *specified*, none is *implemented*: the bodies are `todo!()` or table stubs, and the module is
//! deliberately absent from `super`'s `mod` list so it cannot affect generation. Delete this file
//! when the three kinds land.
//!
//! # The one idea
//!
//! Alef already has exactly one working answer to "assert something that is not a field of the
//! result": a **virtual-field namespace whose accessor reads state the generator arranged around
//! the call**. `streaming_assertions` is the working instance — `collect_snippet` arranges the
//! state (`chunks`), `accessor` reads it, and `is_streaming_virtual_field` intercepts the path
//! *before* `FieldResolver::is_valid_for_result` ever sees it, so the field-availability oracle is
//! never asked a question it cannot answer.
//!
//! The three missing kinds are three more *captures*, not three more result fields:
//!
//! | kind | capture | state at assertion time |
//! |------|---------|-------------------------|
//! | `outcome.*` | the call is invoked in a non-aborting form | a bound outcome/error value |
//! | `stream.*`  | the event sequence is drained into a list  | `chunks` + the event item type |
//! | `timing.*`  | a monotonic clock brackets the call        | an elapsed-milliseconds integer |
//!
//! They are *not* one problem. `stream.*` needs no change to the call's emission shape (the
//! capture already exists in 13 backends); `timing.*` needs a two-statement bracket around the
//! call statement; `outcome.*` changes the call's emission shape outright, because a call that is
//! expected to fail cannot be emitted with the `.expect()` / `try` / propagate form the success
//! path uses.
//!
//! # The prohibition this design is written against
//!
//! None of these may be satisfied by adding a synthetic field to a result type. A synthetic field
//! converts a visible gap into a plausible-looking green. Where a backend cannot express a kind,
//! the required output is a **registered skip wording** — a `field_skip::FieldSkip` or
//! `assertion_type_skip::AssertionTypeSkip` variant, so `fail_on_unavailable_field_markers` /
//! `fail_on_unsupported_assertion_type_markers` count it by construction — and never an
//! `assert!(true)`, never an empty render, never a `return` that emits nothing.

use crate::e2e::codegen::field_skip::SkipClass;
use crate::e2e::fixture::{Assertion, Fixture};

// ---------------------------------------------------------------------------------------------
// KIND 1 — `outcome.*`: the call's outcome as a first-class assertable value.
// ---------------------------------------------------------------------------------------------

/// The canonical virtual-field names in the call-outcome namespace.
///
/// ~keep `is_error` is registered as a *legacy alias* of `outcome.errored`, exactly the way
/// `chunks` is a legacy alias of `stream.items` in `streaming_assertions::model`'s
/// `STREAMING_VIRTUAL_FIELDS`. That is the precedent, and it is what keeps the 43 measured
/// `is_error` fixtures working without a fixture edit. It is emphatically NOT a synthetic result
/// field: like the streaming names, it is intercepted before `is_valid_for_result` is consulted,
/// so the availability oracle is never asked about a path it has no basis to answer.
///
/// `outcome.ok` exists so `is_true`/`is_false` polarity is expressible in both directions without
/// each backend having to render a negation of an arbitrary expression.
pub const OUTCOME_VIRTUAL_FIELDS: &[&str] = &["outcome.errored", "outcome.ok", "is_error"];

/// Whether `field` names the call-outcome namespace.
///
/// Mirrors `streaming_assertions::is_streaming_virtual_field`. Backends call this at the same
/// point in `render_assertion` they already call that one — before the `is_valid_for_result`
/// guard, after the traversal/wildcard guards.
pub fn is_outcome_virtual_field(field: &str) -> bool {
    OUTCOME_VIRTUAL_FIELDS.contains(&field)
}

/// Whether the fixture needs the outcome capture arranged around its call.
///
/// ~keep This is the widened form of the predicate every backend currently spells for itself as
/// `assertions.iter().any(|a| a.assertion_type == "error")` (`codegen/mod.rs`'s
/// `declared_error_value`, `go/test_function.rs`, `csharp.rs`, `zig/test_file.rs`,
/// `python/test_function.rs`, `r.rs`, `go/test_file.rs`). Widening it in one shared place is the
/// whole wiring change for kind 1; a backend that keeps its private copy will emit the success-path
/// call shape and then try to assert `outcome.errored` against a variable that does not exist.
pub fn needs_outcome_capture(fixture: &Fixture) -> bool {
    fixture.assertions.iter().any(|assertion| {
        assertion.assertion_type == "error" || assertion.field.as_deref().is_some_and(is_outcome_virtual_field)
    })
}

/// What the generator arranged around the call, handed to the assertion renderer.
///
/// ~keep `error_var` is the entire contract with the in-flight `error.<field>` lane. That lane
/// renders accessors *on an error that has already been matched*
/// (`FieldResolver::accessor_for_error`, driven today only by `rust/assertions.rs`'s
/// `error.` branch). It has no way to produce the binding itself — that is this capture's job.
/// `error_var: None` is the honest signal that the backend proved failure by control flow alone
/// and therefore that `error.<field>` must render its own registered skip rather than reference a
/// name that is not in scope.
#[derive(Debug, Clone)]
pub struct OutcomeBinding {
    /// A language-native expression that evaluates to `true` iff the call failed.
    ///
    /// Examples: `result.is_err()` (rust), `err != nil` (go), `sample_last_error_code() != 0` (c),
    /// `__outcome_error is not None` (python, with the `try`/`except` form below).
    pub errored_expr: String,
    /// A language-native name for the error value, when the capture bound one.
    ///
    /// `None` means the shape only proves failure by control flow (a bare
    /// `with pytest.raises(Exception):`, `assertThrows(...)` whose return value is discarded).
    pub error_var: Option<String>,
    /// Statements the backend must emit *around* the call to establish the two above.
    pub prelude: OutcomePrelude,
}

/// The emission-shape change an outcome capture forces on the call statement.
///
/// ~keep The variants are the two shapes actually measured across the backends, not a taxonomy
/// invented here. `Bind` is the shape a language with a value-carrying failure channel already
/// uses; `Guard` is the shape a language with exceptions uses. Only `Bind` can populate
/// `OutcomeBinding::error_var` on every path — `Guard` can populate it only when the backend's
/// construct names the caught value (`as exc_info`, `assertThrows(...)`'s return).
#[derive(Debug, Clone)]
pub enum OutcomePrelude {
    /// The call already yields a value carrying the outcome; nothing extra is emitted.
    /// `rust` (`Result`), `go` (`err`), `zig` (error union), `c` (last-error code).
    Bind,
    /// The call must be wrapped so the failure is caught rather than propagated, and the caught
    /// value bound to `error_var`. `before` is emitted above the call, `after` below it, and the
    /// call statement itself is re-indented into the block by `indent_step`.
    Guard {
        before: String,
        after: String,
        indent_step: &'static str,
    },
}

/// Per-language table: how to arrange the outcome capture.
///
/// `None` is a real answer, not a placeholder: it means this backend has no way to observe the
/// call's failure as a value, and the caller must emit
/// [`OutcomeSkip::NoOutcomeCaptureInBackend`]'s wording.
pub fn outcome_binding(_lang: &str, _call_expr: &str, _result_var: &str) -> Option<OutcomeBinding> {
    todo!("specified in docs/design/assertion-kinds.md §1.4; per-language table")
}

/// Registered skip wordings for kind 1. Each becomes one arm of the existing
/// `field_skip::field_skip_variants!` macro — NOT a new funnel.
///
/// ~keep The classes are load-bearing and are the reason this is three variants rather than one.
/// A consumer can fix `WrongPolarityForGuardShape` from their own fixture; they cannot fix the
/// other two from anywhere, so failing their build on those would only force a blanket opt-out —
/// the silent skip again with extra ceremony (`field_skip::SkipClass`'s own doc makes that call).
pub enum OutcomeSkip {
    /// This backend cannot observe the call's failure as a value at all.
    /// Wording: `outcome assertion on field '<f>' requires a call-outcome capture this backend
    /// does not emit`. Class: [`SkipClass::GeneratorGap`].
    NoOutcomeCaptureInBackend,
    /// The backend proved failure by control flow only, so `outcome.ok` (the negative polarity)
    /// has no expression to negate.
    /// Wording: `outcome assertion on field '<f>' has no value form in a guard-shaped error path`.
    /// Class: [`SkipClass::LanguageLimitation`].
    NoValueFormInGuardShape,
    /// The fixture asserts `outcome.errored` is false on a call the backend emitted in the
    /// error-guard shape, or vice versa — a fixture-fixable contradiction.
    /// Wording: `outcome assertion on field '<f>' contradicts the fixture's declared error
    /// outcome`. Class: [`SkipClass::AuthoringGap`].
    WrongPolarityForGuardShape,
}

impl OutcomeSkip {
    /// The class each variant carries into `fail_on_unavailable_field_markers`.
    pub const fn class(self) -> SkipClass {
        match self {
            Self::NoOutcomeCaptureInBackend => SkipClass::GeneratorGap,
            Self::NoValueFormInGuardShape => SkipClass::LanguageLimitation,
            Self::WrongPolarityForGuardShape => SkipClass::AuthoringGap,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// KIND 2 — `stream.*`: assertions over the yielded event sequence.
// ---------------------------------------------------------------------------------------------

/// Everything a streaming accessor needs, in one struct.
///
/// ~keep Replaces the four positional arguments of
/// `StreamingFieldResolver::accessor_with_streaming_context`. The positional form is why eight
/// backends still call the two-argument `accessor(...)` shim: adopting the context meant threading
/// two more parameters through their own `render_assertion` signature, so they did not, and
/// `item_type` silently arrives as `None` — which makes every `stream.has_*_event` accessor return
/// `None`. A struct with one call-site change per backend removes that incentive.
#[derive(Debug, Clone, Copy)]
pub struct StreamingContext<'a> {
    /// Local holding the collected event list.
    pub chunks_var: &'a str,
    /// Local holding the raw stream, before collection.
    pub stream_var: &'a str,
    /// Cargo crate name (rust) or C# namespace — needed to spell the union type path.
    pub module_qualifier: Option<&'a str>,
    /// Unqualified name of the streaming union item type, from
    /// `recipe::streaming_item_type` (explicit `[crates.e2e.call.streaming] item_type`, else the
    /// matching `[[crates.adapters]] pattern = "streaming"` entry's `item_type`).
    pub item_type: Option<&'a str>,
}

/// Why a streaming accessor could not be produced.
///
/// ~keep This three-way split is the substance of kind 2. Today every one of these collapses into
/// a single `Option::None` at the accessor boundary, and the call sites turn that `None` into
/// either one under-specified `GeneratorGap` wording or — in six backends — nothing at all. The
/// split matters because exactly one of the three is fixable by the consumer, from their own
/// `alef.toml`, and the current wording cannot tell them so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamAccessorGap {
    /// An event-variant predicate was asked for but no stream item type resolved.
    /// FIXABLE: set `[crates.e2e.call.streaming] item_type = "..."`, or give the call a matching
    /// `[[crates.adapters]]` with `pattern = "streaming"`. Class: [`SkipClass::AuthoringGap`].
    NoItemType,
    /// This language has no arm in `has_event_variant_accessor`. Class: [`SkipClass::GeneratorGap`].
    NoLanguageAccessor,
    /// The binding does not deliver a typed event sequence at all — PHP's crawl stream arrives as
    /// eager JSON, WASM has no streaming on `wasm32`. Class: [`SkipClass::LanguageLimitation`].
    NotStreamable,
}

/// The fallible replacement for `StreamingFieldResolver::accessor_with_streaming_context`.
///
/// ~keep Returning `Result` rather than `Option` is the point: an `Option` cannot carry the reason,
/// and a caller holding no reason cannot render a wording that distinguishes the three. The
/// existing `Option`-returning entry points stay as shims during the migration and are deleted
/// once the last backend adopts the context.
pub fn stream_accessor(
    _field: &str,
    _lang: &str,
    _context: &StreamingContext<'_>,
) -> Result<String, StreamAccessorGap> {
    todo!("specified in docs/design/assertion-kinds.md §2.3; wraps today's accessors::* arms")
}

// ---------------------------------------------------------------------------------------------
// KIND 3 — `timing.*`: wall-clock properties of the call.
// ---------------------------------------------------------------------------------------------

/// The canonical virtual-field names in the timing namespace.
///
/// ~keep Deliberately neutral. The measured fixtures spell this `rate_limit.min_duration_ms`,
/// which names a consumer domain concept and must never appear in alef —
/// `tests/cli_no_project_special_casing.rs` exists to keep consumer vocabulary out of the
/// generator, and a `rate_limit.*` alias would be exactly the special-casing it forbids. The
/// consumer's half of this kind is a one-line fixture rename to `timing.elapsed_ms`; alef's half is
/// the capture. Until they rename, `rate_limit.min_duration_ms` keeps failing the availability
/// oracle as an unacknowledged `AuthoringGap` — which is fatal under strict, and correct.
pub const TIMING_VIRTUAL_FIELDS: &[&str] = &["timing.elapsed_ms"];

/// The three fragments a backend emits to make `timing.elapsed_ms` readable.
///
/// `start` goes immediately above the call statement, `stop` immediately below it — the same two
/// positions `StreamingFieldResolver::collect_snippet`'s output already occupies for streaming, so
/// the wiring lands at call sites that already exist rather than at new ones.
#[derive(Debug, Clone)]
pub struct TimingCapture {
    pub start: &'static str,
    pub stop: &'static str,
    /// Expression yielding elapsed whole milliseconds as an integer.
    pub elapsed_ms_expr: &'static str,
}

/// Per-language monotonic-clock table.
///
/// ~keep Monotonic, never wall-clock-of-day: a `timing.elapsed_ms` assertion that an NTP step can
/// fail is a flaky test, and a flaky test gets skipped, which lands right back at inert. Every arm
/// must name its language's monotonic source (`Instant`, `time.monotonic`, `CLOCK_MONOTONIC`,
/// `System.nanoTime`, `Stopwatch`, `performance.now`, `hrtime`, `std.time.Timer`), never
/// `SystemTime` / `Date.now` / `time.time`.
pub fn timing_capture(_lang: &str) -> Option<TimingCapture> {
    todo!("specified in docs/design/assertion-kinds.md §3.4; per-language table")
}

/// The one registered wording kind 3 needs.
///
/// Wording: `timing assertion on field '<f>' requires a wall-clock capture this backend does not
/// emit`. Class: [`SkipClass::GeneratorGap`] — a consumer cannot add a clock to alef's emitter
/// from their own config.
pub struct TimingCaptureNotSupported;

// ---------------------------------------------------------------------------------------------
// Shared: the recipe gate.
// ---------------------------------------------------------------------------------------------

/// New `assertion_recipes` opt-in names, alongside the existing
/// `STREAMING_RECIPE` / `CHUNKS_RECIPE` / `EMBEDDINGS_RECIPE` / `KEYWORDS_RECIPE` / `TREE_RECIPE`.
///
/// ~keep `timing` is gated because capturing a clock changes the emitted call and introduces the
/// only source of nondeterminism in the whole suite; that must be a decision someone made, not
/// something a field name turns on. `outcome` is deliberately NOT gated: the 43 measured fixtures
/// already exist and already mean what they say, and adding a required opt-in would convert 43
/// visible skips into 43 generation failures on the first regeneration — a worse signal, not a
/// better one.
pub const TIMING_RECIPE: &str = "timing";

/// Which recipe, if any, a field in one of the three new namespaces requires.
/// Extends `assertion_recipes::required_field_recipe`.
pub fn required_recipe_for_new_kinds(field: &str) -> Option<&'static str> {
    TIMING_VIRTUAL_FIELDS.contains(&field).then_some(TIMING_RECIPE)
}

/// Whether an assertion belongs to any of the three new kinds — the single predicate the shared
/// gate in `E2eCodegen::generate_gated` consults.
pub fn names_a_new_kind(assertion: &Assertion) -> bool {
    assertion.field.as_deref().is_some_and(|field| {
        is_outcome_virtual_field(field)
            || TIMING_VIRTUAL_FIELDS.contains(&field)
            || crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(field)
    })
}
