# Three missing assertion kinds

Status: **specification only**. Nothing in this document is implemented. The type-level half lives
in `src/e2e/codegen/assertion_kinds_design.rs`, which is deliberately not listed in
`src/e2e/codegen/mod.rs` and therefore cannot affect generation.

Nothing here was compiled or regenerated. See "What was not verified" at the end.

---

## 0. The frame

A peer measured, across a consumer's 16-language e2e suite after driving the strategy and filter
suites to zero skipped assertions, what remains inert:

```text
43  is_error
16  stream.has_page_event
16  stream.has_complete_event
16  rate_limit.min_duration_ms
 6  stream.event_count_min
 3  stream.has_error_event
 1  metadata.headings.length
 1  hreflang[].lang / hreflang.length / headings.length / favicons.length  (swift only)
```

These are three problems, not one. Alef already has exactly one working answer to "assert something
that is not a field of the result": **a virtual-field namespace whose accessor reads state the
generator arranged around the call.** `streaming_assertions` is the working instance —
`collect_snippet` arranges the state (`chunks`), `accessor` reads it, and
`is_streaming_virtual_field` intercepts the path *before* `FieldResolver::is_valid_for_result` sees
it, so the availability oracle is never asked a question it has no basis to answer.

Each gap needs a **different capture**:

| kind | capture the generator must arrange | state at assertion time | changes call emission shape? |
|---|---|---|---|
| `outcome.*` (43) | invoke the call in a non-aborting form | a bound outcome / error value | **yes** |
| `stream.*` (41) | drain the event sequence | `chunks` **plus the event item type** | no — capture exists |
| `timing.*` (16) | bracket the call with a monotonic clock | an elapsed-ms integer | no — two statements |

### The prohibition

None of these may be satisfied by adding a synthetic field to a result type. A synthetic field turns
a visible gap into a plausible-looking green. Where a backend cannot express a kind, the required
output is a **registered skip wording** — a `field_skip::FieldSkip` or
`assertion_type_skip::AssertionTypeSkip` variant, so `fail_on_unavailable_field_markers` /
`fail_on_unsupported_assertion_type_markers` count it by construction. Never `assert!(true)`, never
an empty render, and — see §2.2 — never a `return` that emits nothing.

### Why the existing funnels, not a new one

Both funnels already generate their `ALL` recognition set from the *same macro arm* as the variant
list (`field_skip.rs:56`, `assertion_type_skip.rs:32`), so a wording cannot exist without the strict
gate counting it. Both already carry the exact three-way vocabulary these gaps need
(`SkipClass::{AuthoringGap, GeneratorGap, LanguageLimitation}`, `field_skip.rs:44`). A third funnel
would need its own negative-control test set and would fragment `skip_summary`
(`codegen/mod.rs:375`). Every wording below is a new arm of an existing macro.

The strict-gate error text at `codegen/mod.rs:508` **already names all three of these kinds** as the
things `"skip": {"kind": "not_representable"}` is for — "an assertion *kind* such as \"the call
errored\", a property of the call rather than the result, or an assertion over a stream's events".
This work is the implementation of a contract the error message already advertises.

---

## 1. Kind 1 — `is_error` → the `outcome.*` capture

### 1.1 What the fixtures mean

`is_error` is not a field of any type in the consumer's crate. The fixtures mean *the call returned
an error*. Today such an assertion resolves nowhere and lands as
`FieldSkip::NotAvailableOnResultType` — an **`AuthoringGap`**, i.e. fatal under strict, i.e. the
consumer is being told to fix something they cannot fix. `codegen/mod.rs:673` already contains a
unit test that uses exactly this string and exactly this reason
(`"`is_error` is an assertion kind, not a field path"`), so the diagnosis is already recorded in
the codebase; only the capability is missing.

### 1.2 Config surface

A new virtual-field namespace, `outcome.*`:

| field | type | meaning |
|---|---|---|
| `outcome.errored` | bool | the call failed |
| `outcome.ok` | bool | the call succeeded |
| `is_error` | bool | **legacy alias** of `outcome.errored` |

The alias is the same device `streaming_assertions/model.rs:36` already uses to keep `chunks`
working as a legacy spelling of `stream.items`. It is *not* a synthetic result field: like the
streaming names it is intercepted before `is_valid_for_result` is consulted.

`outcome.ok` exists so `is_true`/`is_false` polarity is expressible in both directions without each
backend having to render a negation of an arbitrary expression.

**No recipe gate.** The 43 measured fixtures already exist and already mean what they say; requiring
an opt-in would convert 43 visible skips into 43 generation failures on the first regeneration — a
worse signal, not a better one.

The existing `{"type": "error"}` assertion type stays exactly as it is. It carries semantics
`outcome.errored` does not (an optional `value` matched against message-**or**-class-name; see
`declared_error_value` at `codegen/mod.rs:206` for why that disjunction is load-bearing) and it is
fixture-level rather than per-assertion. `outcome.errored` is the per-assertion predicate; `error`
is the fixture-level declaration. A fixture may carry both.

### 1.3 IR / state needed at emission time

```rust
struct OutcomeBinding {
    errored_expr: String,          // language-native bool expression
    error_var: Option<String>,     // language-native name for the error value
    prelude: OutcomePrelude,       // Bind | Guard { before, after, indent_step }
}
```

`OutcomePrelude::Bind` — the call already yields a value carrying the outcome (Rust `Result`, Go
`err`, Zig error union, C last-error code). Nothing extra is emitted.

`OutcomePrelude::Guard` — the call must be wrapped so failure is caught rather than propagated
(Python `try`/`except`, Java `assertThrows`, Ruby `expect { }.to raise_error`, JS `.rejects`), and
the caught value bound to a name.

### 1.4 Where it hooks

Three edits per backend, all at sites that already exist:

1. **The predicate.** Every backend spells "this fixture expects an error" for itself as
   `assertions.iter().any(|a| a.assertion_type == "error")`. The live copies are
   `codegen/mod.rs:210`, `python/test_function.rs` (`has_error_assertion`),
   `go/test_function.rs:214`, `go/test_file.rs:27`, `csharp.rs:454`, `zig/test_file.rs:222`,
   `zig/test_file.rs:639`, `r.rs:160`, `r.rs:184`, `validate.rs:169`, `fixture.rs:954`. Widen it in
   **one** shared helper (`needs_outcome_capture`). A backend that keeps its private copy will emit
   the success-path call shape and then assert against a variable that does not exist.

2. **The call emission.** Where the backend already branches on that predicate to choose between
   `.expect()`-style and guard-style emission. Rust: `rust/test_file/test_function.rs:545`
   (`unwrap_suffix`) and `:522` (`result_binding`). Python: `python/test_function.rs:257`. Go:
   `go/test_function.rs:214`. C: `c/streaming.rs:233` and `c.rs:2004`.

3. **The assertion dispatch.** A new `is_outcome_virtual_field(f)` interception at the same point in
   `render_assertion` each backend already calls `is_streaming_virtual_field` — before the
   `is_valid_for_result` guard, after the traversal/wildcard guards. Named per backend:
   `rust/assertions.rs:242`, `go/assertions.rs:217`, `java/assertions.rs:~287`,
   `python/test_function/result_assertions.rs:166`, `typescript/assertions.rs:300`,
   `swift/assertions.rs:44`, `elixir/assertions.rs:207`, `php/assertions.rs:160`,
   `kotlin/assertions.rs:107`, `zig/assertions.rs:336`, `ruby/examples.rs:253`,
   `csharp/streaming.rs`, `c/streaming.rs:311`.

### 1.5 Composition with the in-flight `error.<field>` lane

They are adjacent and must not duplicate. Read the seam at `assertion_type_skip.rs:106`
(`EqualsOnErrorFieldNotSupported`) and the wiring model at `python/test_function.rs:269`.

**Division of ownership — one string:**

- `outcome.*` **owns the capture.** It arranges the call so an error value exists and is bound, and
  it publishes the binding's name.
- `error.<field>` **owns the accessor.** It reads a field off an error value that is already in
  scope, via `FieldResolver::accessor_for_error(sub, lang, err_var)` — driven today only by
  `rust/assertions.rs:486`, which inline-binds `__err` itself:
  `{ let __err = result.as_ref().err().unwrap(); <accessor> }`.
- The contract between them is `OutcomeBinding::error_var: Option<String>`.
  `Some(name)` → `error.<field>` renders against `name`. `None` → the backend proved failure by
  control flow alone, and `error.<field>` renders `EqualsOnErrorFieldNotSupported` — the wording it
  already owns. **`outcome.*` must not register a duplicate of that wording**, and
  `error.<field>` must not invent its own error binding.

Two concrete consequences an implementer must not miss:

- Python **already** has half the binding: `error_assertions.rs`'s `has_message` branch emits
  `with pytest.raises(Exception) as exc_info:` (a name), but the no-message branch emits a bare
  `with pytest.raises(Exception):` (no name). That asymmetry means `error.<field>` is expressible on
  one branch and not the other for reasons that have nothing to do with the fixture. Unifying it is
  a one-line change **in the `error.<field>` lane's own file** — specified here, not made here.
- `render_unrenderable_error_path_assertions` (`error_assertions.rs`) currently emits a skip for
  *every* assertion beyond the primary `error` check. Once `outcome.*` lands, an
  `is_true`/`outcome.errored` assertion on an error-path fixture must be **rendered**, not skipped —
  so that function needs a carve-out for outcome fields. Failing to add it turns a newly-working
  assertion back into a counted skip, which looks like progress in the summary and is not.

**Order dependency:** `outcome.*` should land *after* the `error.<field>` lane merges, so it consumes
that lane's binding contract rather than racing it.

### 1.6 Skip wordings (new `field_skip_variants!` arms)

| variant | class | wording (after `skipped: `) |
|---|---|---|
| `NoOutcomeCaptureInBackend` | `GeneratorGap` | `outcome assertion on field '<f>' requires a call-outcome capture this backend does not emit` |
| `NoValueFormInGuardShape` | `LanguageLimitation` | `outcome assertion on field '<f>' has no value form in a guard-shaped error path` |
| `WrongPolarityForGuardShape` | `AuthoringGap` | `outcome assertion on field '<f>' contradicts the fixture's declared error outcome` |

Only the third is fatal under strict, and only the third is fixable by a fixture edit — which is
exactly the criterion `field_skip.rs:44`'s doc sets for `AuthoringGap`.

---

## 2. Kind 2 — `stream.*` over the event sequence

### 2.1 What is actually missing

Substantially less than it looks, and substantially more than the count says.

The four event sub-kinds are **already registered** as streaming-virtual fields
(`streaming_assertions/model.rs:36`), **already gated** behind the `streaming` recipe
(`assertion_recipes.rs:46`), and **already have per-language accessors** — `has_event_variant_accessor`
(`streaming_assertions/accessors.rs:531`) has working arms for python, node/typescript, ruby, go,
java, csharp, swift, elixir, kotlin, kotlin_android, dart, zig and rust: 13 languages. It returns
`None` for php and wasm *deliberately* (PHP's crawl stream arrives as eager JSON; WASM has no
streaming on `wasm32`), and `None` by fallthrough for everything else.

The blocker is that all three `stream.has_*_event` arms are guarded by
`item_type.and_then(...)` (`accessors.rs:232-238`), and `item_type` arrives as `None` in most
backends because they call the two-argument shim `StreamingFieldResolver::accessor(...)` instead of
`accessor_with_streaming_context(...)`. Measured call sites:

- **Threading `item_type`:** `go/assertions.rs:224`, `rust/assertions.rs:246`,
  `java/assertions.rs:290`, `ruby/examples.rs:253`.
- **Not threading it** (so `item_type` is structurally `None` and every `stream.has_*_event`
  accessor returns `None`): `python/test_function/result_assertions.rs:169`,
  `typescript/assertions.rs:303`, `swift/assertions.rs:49`, `elixir/assertions.rs:207`,
  `php/assertions.rs:160`, `zig/assertions.rs:336`, `kotlin/assertions.rs:40` and `:107`.
- **Never calling `StreamingFieldResolver` at all** — they hand-roll their own streaming dispatch,
  so no accessor is reachable regardless of `item_type`: `csharp` (own field match at
  `csharp/streaming.rs:409` and `:472-481`, despite resolving an item type at `:13-25` and despite
  `accessors.rs:560` having a C# arm ready), `dart` (`dart/test_case.rs:1066`), `c`
  (`c/streaming.rs:320-327`).

Note the shape of that middle group: **swift and zig both resolve an `item_type` and feed it only to
the collect snippet** (`swift/test_method.rs:134-139`, `zig/test_file.rs:262`), then call the bare
accessor. Their `has_event_variant_accessor` arms (`accessors.rs:567`, `:584`) are written, tested
and unreachable. So is C#'s. That is three languages where the capability exists end-to-end except
for one argument.

`recipe::streaming_item_type` (`recipe.rs:289`) resolves it from
`[crates.e2e.call.streaming] item_type` first, else the matching `[[crates.adapters]]` with
`pattern = "streaming"`. **That is consumer-fixable from `alef.toml`.**

### 2.2 The finding that changes the count

When the accessor returns `None`, **nine** backends emit nothing at all and return, and a separate
overlapping set of **six** emits a wording **neither funnel recognises**.

*Silent-on-`None`* — the `if let Some(expr) = ...` has no `else`, and the enclosing block `return`s
regardless, so the assertion evaporates with no line for either gate to see. **Nine backends:**

| backend | site | evidence |
|---|---|---|
| go | `go/assertions.rs:217-311` | verified: `return;` at :310 is at the outer `if`'s body indent (opened :222, closed :311), *outside* the `if let` at :223-309 |
| typescript (`node`) | `typescript/assertions.rs:302-371` | verified: the arm falls through to `true` ("handled") having written nothing |
| wasm | same renderer — `wasm.rs:380` → `typescript::render_test_file` | verified by delegation |
| swift | `swift/assertions.rs:48-115` | verified: `return;` at :115, outside the `if let` at :49 |
| elixir | `elixir/assertions.rs:206-262` | verified: `}` closing the `if let`, then bare `return;` |
| zig | `zig/assertions.rs:336-376` | verified: same shape |
| php | `php/assertions.rs:159-~222` | agent-reported, same shape, not line-verified here |
| kotlin | `kotlin/assertions.rs:106-181` | agent-reported, same shape, not line-verified here |
| kotlin_android | same renderer (`kotlin/test_method.rs:194-198` selects `stream_lang`) | agent-reported |

A subagent initially reported go as falling through to the `NotAvailableOnResultType` skip at
`go/assertions.rs:318`. That is wrong — the brace nesting was re-read line by line and the `return;`
is inside the streaming block. Recorded because the same misreading is easy to repeat.

*Unregistered wording* — `// streaming field '<f>': assertion type '<t>' not rendered`, emitted by
**six** backends: `go/assertions.rs:304`, `typescript/assertions.rs:365`, `java/assertions.rs:352`,
`kotlin/assertions.rs:172`, `php/assertions.rs:216`, `swift/assertions.rs:106`. Checked against
every registered `Shape`: `FieldSkip::NoPythonStreamingAccessor` is `("streaming field ", ": no
python accessor")` — the suffix does not match; `AssertionTypeSkip::StreamingAssertionTypeNotSupported`
is `("assertion type ", " on field '")` — the suffix does not match. Neither funnel counts it. The
line also lacks the `skipped:` prefix, so even a naive grep misses it.

Counter-examples that *do* work, and are the models: `python/test_function/result_assertions.rs:171`
renders `FieldSkip::NoPythonStreamingAccessor` (counted, `GeneratorGap`);
`c/streaming.rs:365` renders `FieldSkip::StreamingAssertionOnUnsupportedField` (counted);
`rust/assertions.rs:373` **panics** with the item type in the message (loudest of all).

**Therefore the measured 41 is a floor, not a total.** Backends that emit nothing contribute zero to
the peer's inventory while asserting nothing. A `stream.has_page_event` assertion in a Go, Swift,
Zig, Elixir, PHP, Kotlin or TypeScript fixture today produces a generated test that is green,
compiles, and checks nothing — and there is no line anywhere for `ALEF_E2E_STRICT_ASSERTIONS` to
scan. This is the single most important claim in this document and it is why step 0 of §6 exists.

It also means this document cannot tell you the true size of kind 2, and neither can the peer's
inventory. Only a regeneration after step 0 can. Do not treat 41 as a target.

### 2.3 The design

Replace the four positional arguments with one struct, and replace `Option` with `Result` so the
reason survives to the call site:

```rust
struct StreamingContext<'a> {
    chunks_var: &'a str,
    stream_var: &'a str,
    module_qualifier: Option<&'a str>,   // cargo crate (rust) / C# namespace
    item_type: Option<&'a str>,          // from recipe::streaming_item_type
}

enum StreamAccessorGap {
    NoItemType,          // AuthoringGap      — fixable from alef.toml
    NoLanguageAccessor,  // GeneratorGap      — alef owes an arm
    NotStreamable,       // LanguageLimitation — php eager-JSON, wasm32
}

fn stream_accessor(field, lang, &StreamingContext) -> Result<String, StreamAccessorGap>;
```

The positional form is *why* eight backends never adopted the context: adopting it meant threading
two more parameters through their own `render_assertion` signature. A struct makes it one call-site
change each.

The `Option` → `Result` change is the substance. Today all three reasons collapse into one `None`,
and exactly one of the three — `NoItemType` — is something the consumer can fix from their own
`alef.toml`. Under the current single `GeneratorGap` wording it is attributed to alef and never
fatal, so a consumer who could have fixed it is never told.

`stream.event_count_min` needs no accessor work (`accessors.rs:243` resolves for every language) but
does need the assertion **type** used with it to exist in each backend's streaming match arm. Go's
arms are `count_min, count_equals, equals, not_empty, is_empty, is_true, is_false, greater_than,
greater_than_or_equal, contains` (`go/assertions.rs:233-300`); anything else hits the unregistered
default arm at :304.

### 2.4 Skip wordings (new `field_skip_variants!` arms)

| variant | class | wording |
|---|---|---|
| `StreamingItemTypeNotConfigured` | `AuthoringGap` | `streaming event assertion on field '<f>' requires a configured stream item_type` |
| `StreamingEventVariantNotAccessible` | `GeneratorGap` | `streaming event assertion on field '<f>' has no event-variant accessor in this backend` |
| `StreamNotDeliverableInBinding` | `LanguageLimitation` | `streaming assertion on field '<f>' requires a stream this binding does not deliver` |

Plus one `assertion_type_skip_variants!` arm replacing the six unregistered wordings, keeping the
existing prose so nothing grepping for it breaks:

| variant | class | wording |
|---|---|---|
| `StreamingAssertionTypeNotRendered` | `GeneratorGap` | `streaming field '<f>': assertion type '<t>' not rendered` |

Note the token this one captures is the **field**, matching `UnsupportedAssertionTypeOnSyntheticField`'s
precedent (`assertion_type_skip.rs:76`) — the existing wording puts the field in the first quote
pair and the shape/recognition must read the same string.

`StreamingItemTypeNotConfigured` being `AuthoringGap` makes it **fatal under strict**. That is
deliberate and is the whole point: it is the one streaming gap a consumer owns.

---

## 3. Kind 3 — `rate_limit.min_duration_ms` → the `timing.*` capture

### 3.1 What it is

A wall-clock property of the *call*, not of the result. No field mapping can ever express it,
because the quantity does not exist until the generator measures it.

### 3.2 Config surface — and the naming constraint

The canonical name is **`timing.elapsed_ms`**, and `rate_limit.min_duration_ms` is **not** aliased.

`rate_limit` names a consumer domain concept. `tests/cli_no_project_special_casing.rs` exists to
keep consumer vocabulary out of the generator, and a `rate_limit.*` alias is exactly the
special-casing it forbids. This kind therefore has two halves:

- **alef's half:** provide `timing.elapsed_ms` and the capture.
- **the consumer's half:** rename the fixture field to
  `{"type": "greater_than_or_equal", "field": "timing.elapsed_ms", "value": 900}`.

Until they rename, `rate_limit.min_duration_ms` keeps failing the availability oracle as an
unacknowledged `AuthoringGap` — fatal under strict, and correct: it *is* a fixture edit.

This is the one place where the honest answer is "rename the field", and it must be stated plainly
rather than smoothed over with an alias, because an alias would put a consumer's product vocabulary
into alef's generator permanently.

**Recipe gate: yes.** `timing.*` requires a new `TIMING_RECIPE = "timing"` opt-in in
`assertion_recipes.rs` (alongside `STREAMING_RECIPE` at :18). Rationale: capturing a clock changes
the emitted call and introduces the only source of nondeterminism in the whole generated suite.
That must be a decision someone made, not something a field name switches on. Contrast `outcome.*`
(§1.2), which is deliberately ungated.

### 3.3 IR / state needed at emission time

```rust
struct TimingCapture {
    start: &'static str,            // emitted immediately above the call statement
    stop: &'static str,             // emitted immediately below it
    elapsed_ms_expr: &'static str,  // integer expression the accessor returns
}

fn timing_capture(lang: &str) -> Option<TimingCapture>;
```

One shared table, exactly the shape of `StreamingFieldResolver::collect_snippet`
(`streaming_assertions/snippets.rs:9`).

**Every arm must name its language's monotonic source** — `Instant`, `time.monotonic`,
`CLOCK_MONOTONIC`, `System.nanoTime`, `Stopwatch`, `performance.now`, `hrtime(true)`,
`std.time.Timer`, `Process::CLOCK_MONOTONIC`, `System.monotonic_time`, `DispatchTime.now` — and
never `SystemTime` / `Date.now` / `time.time` / `Sys.time`. A `timing.elapsed_ms` assertion an NTP
step can fail is a flaky test, a flaky test gets skipped, and a skipped test is where this whole
effort started.

### 3.4 Where it hooks

`start`/`stop` occupy the same two positions around the call statement that
`collect_snippet`'s output already occupies for streaming — so the wiring lands at call sites that
already exist. Rust's is `rust/test_file/test_function.rs:551-556` (the `let {stream_var} = {call_expr}...`
line followed by the collect snippet). The equivalent site in each other backend is its
`render_test_function` / `render_test_method` / `test_case` call-emission line.

There is **no shared hook** for this. `E2eCodegen::generate_gated` (`codegen/mod.rs:1079`) is the
only cross-backend seam and it runs before any code is emitted, so kind 3 costs one small edit per
backend. That is the honest cost and it is why kind 3 is sequenced second rather than first.

### 3.5 Skip wording (new `field_skip_variants!` arm)

| variant | class | wording |
|---|---|---|
| `TimingCaptureNotSupported` | `GeneratorGap` | `timing assertion on field '<f>' requires a wall-clock capture this backend does not emit` |

`GeneratorGap`: a consumer cannot add a clock to alef's emitter from their own config.

---

### 3.6 A finding that must land before kind 1 — the template-level drop

Measured while surveying the error paths, and it changes what "counted" means.

Six backends **build the full assertions body into a string, hand it to their template, and the
template's `expects_error` arm never emits it**: typescript (`templates/typescript/test_function.jinja:27`,
body built at `typescript/test_file/test_case.rs:267`), wasm (same renderer), java
(`templates/java/test_method.jinja:20`, body built at `java/test_method.rs:319-383`), csharp
(`templates/csharp/test_method.jinja:27`, body at `csharp.rs:869-889`), php
(`templates/php/test_method.jinja:25`, body at `php/test_method.rs:375-393`), ruby
(`templates/ruby/test_function.jinja:47`, body at `ruby/examples.rs:420-433`).

The consequence: `fail_on_unavailable_field_markers` and
`fail_on_unsupported_assertion_type_markers` scan a body that **no generated test will ever
contain**. Skips inside it are counted — inflating the ledger — while the *rendered* assertions
inside it are silently discarded, which the ledger cannot see at all. Both directions are wrong,
and both are invisible today.

This is not part of any of the three kinds, but kind 1 lands directly on top of it: an
`outcome.errored` assertion rendered into a body the template throws away is a green test that
asserted nothing, which is the exact defect being unwound. **Fix the discard before wiring kind 1
into those six backends**, or verify per-backend that the body reaches the output.

---

## 4. Per-backend feasibility

Legend: **YES** = expressible today with the state named; **WIRE** = the state exists but is not
threaded to the assertion renderer (a wiring change, no new capability); **SKIP** = must emit a
registered visible skip; **N/A** = the backend renders no fixture assertions at all;
**unmeasured** = not checked in this pass.

`php_ext` (`php_ext.rs:32` — `generate()` takes `_groups` unused) and `homebrew`
(`homebrew.rs:51` — ignores fixture groups entirely) emit no fixture-driven assertions of any kind,
so all three columns are N/A for them. `brew` emits shell smoke tests only.

### 4.1 `outcome.*` (kind 1)

`OutcomePrelude::Bind` = a value carrying the outcome already exists. `Guard` = a catch construct.
"error value bound?" is the contract with the `error.<field>` lane (§1.5).

| backend | verdict | prelude | outcome expression available | error value bound? | evidence |
|---|---|---|---|---|---|
| rust | YES | Bind | `result.is_err()` | **yes, always** — `result`, plus `result_ok`, plus inline `__err` | `rust/assertions.rs:505`, `:486`; branch `rust/test_file/test_function.rs:393` |
| go | YES | Bind | `err != nil` | yes — `err` | `go/test_function.rs:214`, branch :367 |
| c | YES | Bind | `result == NULL` / `status != 0` / `{prefix}_last_error_code() != 0` | yes — `result`/`status`, or the global last-error code | `c/test_function.rs:243`, :520, :788, :996, :1312; `c.rs:2004` |
| elixir | YES | Bind | `{:error, __reason} = <call>` | **only when the fixture declares an error value**; otherwise `_` | `elixir/test_case.rs:24`, :27, :30 |
| gleam | YES | Bind | `let assert Error(__reason) = __result` | only when a value is declared; the `should.be_error()` pipe form binds nothing | `gleam/test_case.rs:23`, :26-27, :33 |
| python | WIRE | Guard | `pytest.raises` proves failure | **only when a value is declared** (`as exc_info`); the bare branch binds nothing | `python/test_function/error_assertions.rs:33` vs :66 |
| java | WIRE | Guard | `assertThrows(...)` | only when `declared_error_check` non-empty (`Exception thrown = ...`) | `java/test_method.rs:23`; `templates/java/test_method.jinja:28` |
| csharp | WIRE | Guard | `Assert.ThrowsAnyAsync<...>` | only when declared (`var thrown = ...`) | `csharp.rs:50`, :454; `templates/csharp/test_method.jinja:37` |
| php | WIRE | Guard | `try/catch` or `expectException` | `$e` only in the try/catch (declared-value) form | `php/test_method.rs:53`, :64-75 vs :78 |
| r | WIRE | Guard | `tryCatch(...)` or `expect_error(...)` | `e` only in the declared-value form | `r/test_case.rs:19`, :27 |
| swift | WIRE | Guard | `do { } catch { }` | **yes** — Swift's implicit `error` is in scope for the whole catch body | `swift/test_method.rs:24`, :29-30 |
| ruby | SKIP (`NoValueFormInGuardShape`) | Guard | `expect { }.to raise_error` | block-scoped only (`{ \|error\| }`), unreadable outside | `ruby/examples.rs:26`, :368 |
| typescript | SKIP | Guard | `.rejects.toThrow()` | no — `error` exists only as a `toSatisfy` callback param | `templates/typescript/test_function.jinja:52-60` |
| wasm | SKIP | Guard | same as typescript | no | `wasm.rs:380` |
| dart | SKIP | Guard | `expectLater(..., throwsA(...))` | no — `e` only inside a `predicate` lambda | `dart/test_case.rs:26`, :987, :1011 |
| kotlin | SKIP | Guard | `assertFailsWith<Exception> { }` | no — the returned exception is discarded, `Unit` written instead | `kotlin/test_method.rs:357`, :440 |
| kotlin_android | SKIP | Guard | same renderer | no | `kotlin/test_method.rs:189` |
| zig | SKIP | Bind-by-control-flow | `if (call()) \|_\| return error.TestUnexpectedResult else \|_\| {}` | no — **both** error-union captures explicitly discarded as `\|_\|` | `zig/test_file.rs:382`, :394-396 |
| brew | SKIP | Bind (shell) | `if cmd; then FAIL; fi` exit status | `$output` only in the declared-value form | `brew/category.rs:67`, :95-99 |
| php_ext | N/A | — | — | — | `php_ext.rs:32` |
| homebrew | N/A | — | — | — | `homebrew.rs:51` |

**The blocker every WIRE/SKIP row shares:** twenty of twenty-one backends **drop the fixture's other
assertions entirely on the error path**. Rust alone loops them
(`rust/test_file/test_function.rs:420`). Ten drop via a Rust-level early return (go :378, kotlin
:392/:452, elixir :397/:436, gleam :171/:179, r :165, c :526/:800/:1016/:1320, dart's else-arm at
:1064, swift :406, brew :176, zig's branch chain at :382). Six drop via the template (§3.6). One —
python — emits explicit skip comments (`error_assertions.rs:88`).

So kind 1's real cost is not the outcome expression, which most backends already have. It is
**making the error path render assertions at all**. That is the single largest item in this
document and the reason kind 1 is sequenced last.

### 4.2 `stream.*` (kind 2)

| backend | collect step (var, file:LINE) | item_type resolved (file:LINE) | threaded to accessor? | event-variant accessor arm | verdict |
|---|---|---|---|---|---|
| rust | `chunks` — `rust/test_file/test_function.rs:553-557` | `test_function.rs:452-459` | **YES** `rust/assertions.rs:246` | `accessors.rs:590` (needs crate qualifier) | **YES** |
| go | `chunks` — hand-rolled `go/test_function.rs:497-501` | `go/test_function.rs:386-388` | **YES** `go/assertions.rs:224` | `accessors.rs:551` | **YES** |
| java | `chunks` — `java/test_method.rs:444-453` | `java/test_method.rs:360-361` | **YES** `java/assertions.rs:290` | `accessors.rs:555` | **YES** |
| ruby | `chunks` — hand-rolled `ruby/examples.rs:195,198` | `ruby/spec_file.rs:169-171` | **YES** `ruby/examples.rs:253` | `accessors.rs:547` | **YES** |
| swift | `chunks` — `swift/test_method.rs:456-458` | `swift/test_method.rs:134-139` | **NO** — bare `accessor` `swift/assertions.rs:49` | `accessors.rs:567` | **WIRE** |
| zig | `chunks` + `chunks_content` — `zig/test_file.rs:491-502` | `zig/test_file.rs:262`, used at :499 | **NO** — bare `accessor` `zig/assertions.rs:336` | `accessors.rs:584` | **WIRE** |
| csharp | `chunks` — hand-rolled `csharp/streaming.rs:248,290` | `csharp/streaming.rs:13-25`, `csharp.rs:512` | **NO** — never calls `StreamingFieldResolver` at all | `accessors.rs:560` (needs C# namespace qualifier — never supplied) | **WIRE** |
| python | `chunks` — `python/.../result_assertions.rs:76-79` | unmeasured — no `streaming_item_type` call in `codegen/python/` | **NO** `result_assertions.rs:169` | `accessors.rs:542` | **WIRE + item_type source** |
| typescript | `chunks` — `typescript/test_file/test_case.rs:314-318` | unmeasured | **NO** `typescript/assertions.rs:303` | `accessors.rs:545` | **WIRE + item_type source** |
| kotlin | `chunks` — `kotlin/test_method.rs:199-207` | unmeasured | **NO** `kotlin/assertions.rs:107` | `accessors.rs:575` | **WIRE + item_type source** |
| kotlin_android | same | unmeasured | **NO** same site | `accessors.rs:577` | **WIRE + item_type source** |
| elixir | `chunks_var` — `elixir/test_case.rs:543-551` | unmeasured | **NO** `elixir/assertions.rs:207` | `accessors.rs:571` | **WIRE + item_type source** |
| dart | awaited `.toList()` bound to `result` — `dart/test_case.rs:1055-1059` | unmeasured | **NO** — never calls `StreamingFieldResolver` | `accessors.rs:579` | **WIRE + item_type source** |
| php | `$chunks` — `php/test_method.rs:325-330` | unmeasured | **NO** `php/assertions.rs:160` | **none** — explicit `"php" \| "wasm" => None` `accessors.rs:596` (eager-JSON stream) | **SKIP** `StreamNotDeliverableInBinding` |
| wasm | `chunks` — shared TS renderer, `snippets.rs:94-96` | unmeasured | **NO** | **none** — same explicit arm | **SKIP** `StreamNotDeliverableInBinding` (no streaming on `wasm32`) |
| c | **no list** — only `size_t chunks_count` `c/streaming.rs:251`, chunks freed at :286 | `c/streaming.rs:18-36`, used at :130-131 for `_free` symbol names | **NO** — never calls `StreamingFieldResolver` | **none** (`_ => None`) | **SKIP** `StreamingEventVariantNotAccessible` — a count is not a sequence; a per-event predicate needs the chunk retained |
| gleam | none — no `resolve_is_streaming` anywhere under `gleam/` | none | n/a | **none** | **SKIP** |
| r | none — only non-streaming synthetic `chunks_have_*` (`r/assertions.rs:27,42,59`) | none | n/a | **none** | **SKIP** |
| brew | none | none | n/a | **none** | **N/A** |
| php_ext | none | none | n/a | **none** | **N/A** |
| homebrew | none | none | n/a | **none** | **N/A** |

`stream.event_count_min` is separable and much cheaper: `accessors.rs:243-257` resolves a length
expression for every language with explicit arms for java/go/php/kotlin/python/rust/node/swift/zig/
ruby/elixir/c plus a `_ => .length` default, and needs **no** `item_type`. The 6 measured
`stream.event_count_min` skips are therefore an assertion-**type** problem (whether
`greater_than_or_equal` has an arm in that backend's streaming match), not an accessor problem.

Only **4 of 21** backends can produce a `stream.has_*_event` accessor today. **9 of 21** emit
nothing when they cannot (§2.2). That is the shape of the real debt.

### 4.3 `timing.*` (kind 3)

Every backend that renders fixture assertions at all can express an elapsed-milliseconds capture —
this is the one kind with essentially no language-level obstacle. The cost is one bracket at each
backend's call-emission site (the same sites listed in §4.2's collect-step column).

| backend | monotonic source | verdict |
|---|---|---|
| rust | `std::time::Instant` | YES |
| python | `time.monotonic()` | YES |
| typescript / wasm | `performance.now()` | YES |
| go | `time.Now()` / `time.Since` | YES |
| java / kotlin / kotlin_android | `System.nanoTime()` | YES |
| csharp | `System.Diagnostics.Stopwatch` | YES |
| php | `hrtime(true)` | YES |
| ruby | `Process.clock_gettime(Process::CLOCK_MONOTONIC)` | YES |
| elixir | `System.monotonic_time(:millisecond)` | YES |
| swift | `DispatchTime.now().uptimeNanoseconds` | YES |
| dart | `Stopwatch` | YES |
| c | `clock_gettime(CLOCK_MONOTONIC, ...)` | YES |
| zig | `std.time.Timer` | YES |
| r | `Sys.time()` is **not** monotonic; `proc.time()`/`nanotime` needed | unmeasured — pick a monotonic source or SKIP |
| gleam | `erlang:monotonic_time` via FFI | unmeasured |
| brew | shell `SECONDS` / `date +%s%N` (not portable to macOS `date`) | unmeasured — likely SKIP |
| php_ext / homebrew | renders no fixture assertions | N/A |

Rows marked unmeasured are **unmeasured**, not unsupported: the language almost certainly has a
monotonic clock; nobody checked which one this generator can reach.

---

## 5. The four Swift singles

`hreflang[].lang`, `hreflang.length`, `headings.length`, `favicons.length` (swift only) and
`metadata.headings.length` are first-class-map limitations already understood — see
`swift/snippet.rs:53` (`build_swift_first_class_map`), `swift/accessors.rs:138`
(`swift_is_first_class`) and `FieldSkip::ExcludedFromSwiftBinding` /
`CrossesTaggedUnionBoundaryInSwift`. Recorded here for completeness; **not designed for**. They are a
different axis (which Swift type shape a field lands in) and belong to whichever lane owns
`swift/assertions.rs`.

---

## 6. Implementation order

**0 — close the two silent holes first. Not a kind; a precondition.**
Nine backends emit *nothing* when a streaming accessor returns `None` (§2.2) and six emit a wording
neither funnel recognises (§2.2). Six backends render a whole assertions body and let the template
discard it (§3.6). Until those are closed, every measurement of every kind below is a floor rather
than a total, and any "we fixed N" claim is unfalsifiable. This step adds no capability at all — it
only makes the existing gaps countable — which is exactly why it must not be folded into step 1
and quietly deprioritised inside it.

**1 — `stream.*` first.**
Highest ratio of built machinery to new code: 13 of the per-language event-variant accessors already
exist (`accessors.rs:531-599`) and no call-emission shape changes. Only 4 of 21 backends thread an
`item_type`, so the majority of the work is one call-site change each, not new code. Doing it first
also proves out the `StreamingContext` + `Result`-carrying-a-reason pattern cheaply, before two
other kinds copy it. **Blocked on:** the four lanes currently editing `src/e2e/codegen/**` must
merge first; this touches the same dispatch.

**2 — `timing.*` second.**
The most mechanically isolated of the three. Brand-new namespace behind a brand-new recipe gate, no
interaction with error paths or streams, and no existing behaviour changes — nothing can regress,
because nothing reaches the new code until a fixture opts in. Sequencing it second gets a full kind
delivered while kind 1's dependency clears.

**3 — `outcome.*` last.**
The only kind that changes the call's emission shape, so the only one where a wrong lowering can
turn a genuinely failing test green. It must land **after** the in-flight `error.<field>` lane so it
consumes that lane's error-binding contract (§1.5) instead of racing it. And its true cost is not
the outcome expression — most backends already hold one (§4.1) — but the fact that **twenty of
twenty-one backends drop the fixture's other assertions entirely on the error path**. Rust alone
loops them. That is a per-backend structural change, and step 0 must precede it or the six
template-drop backends will render `outcome.errored` into a string nobody emits.

Counter-argument considered and rejected: `is_error` is the largest single count (43) and could be
argued first on impact. But 43 `is_error` skips are *already visible and already counted* as
`AuthoringGap`s, so they are the best-signalled of the three today — and being `AuthoringGap`s they
are also *fatal under strict*, which means they are already forcing attention. The streaming skips
are the worst-signalled: nine backends emit nothing at all. Fix the invisible one first.

---

## What was not verified

- **Nothing was compiled.** `cargo check` / `cargo build` / `cargo test` were not run, on the
  instruction that only the orchestrator compiles. `src/e2e/codegen/assertion_kinds_design.rs`
  passes `rustfmt --check --edition 2024` and nothing more; its `use` paths and trait bounds are
  unverified.
- **No regeneration was run.** No `alef` invocation, no e2e tree was produced or measured. Every
  claim about generated output is read off the generator source, not off a generated file.
- **The peer's inventory was not re-measured.** The 43/16/16/16/6/3/1/1 counts are taken as given —
  and §2.2 argues they are a floor.
- **The feasibility tables in §4 cover all 21 generators in `all_generators()`
  (`codegen/mod.rs:1129`), not the consumer's 16.** Which 16 the consumer enables was not measured.
  `brew`, `homebrew` and `php_ext` are almost certainly not among them.
- **Provenance of the §4 rows is mixed.** Rows and line numbers marked "verified" were re-read
  directly. The rest came from two read-only survey agents and were spot-checked, not re-derived.
  One agent claim was checked and found **wrong** (go's silent-on-`None`, §2.2); assume others of
  the same shape may be, and re-read the specific site before editing it.
- **Anything marked `unmeasured` was not checked, and must not be read as "absent"** —
  `field_skip.rs`'s own module history is the argument for keeping that distinction.
- **The `timing.*` per-language clock table (§4.3) is proposed, not validated.** No snippet was
  compiled in any target language.
