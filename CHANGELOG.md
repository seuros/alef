# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Fixture documentation can now select a safe relative snippet output path per configured target while retaining the
  shared topic/stem fallback.

- Snippet compile sessions now wire configured Rust crates, TypeScript packages, C headers, Swift packages, and Zig
  modules into isolated validator projects instead of using their manifests only for cache fingerprints.

- Snippet sessions now canonicalize configured package paths before changing subprocess directories; Rust scratch
  crates establish their own workspace boundary, and Swift compilation discovers package C-module maps.

- Binding-aware snippet sessions now resolve Go and Dart package manifests from their actual project roots, with
  regression coverage for local Go, C#, Dart, Java, and Kotlin dependencies.

- Strict documentation coverage now treats paths from a current, complete fixture-snippet ledger as authoritative
  references while rejecting missing files and stale ledger formats.

- Go mock-server integration fixtures now initialize the complete protocol fixture surface.

- Generated snippets now retain their binding target independently of the canonical fence language, allowing Node and
  WASM or Kotlin and Kotlin Android examples to resolve distinct validation sessions.

- PHP no longer reports complex map fields as non-settable when serde-backed `fromJson()` construction is available.

- Poly's successful "files reformatted" status no longer produces a non-fatal formatter warning during scaffolding.

- C documentation snippets now reuse engine-factory and byte-buffer call preparation, while C, Swift, and Zig
  streaming snippets reuse their binding-aware E2E call paths instead of leaving fixture-language coverage gaps.

- Snippet validation sessions now apply explicitly configured environment variables to setup and validation commands,
  allowing tool caches to work without inheriting ambient user environment state.

- Snippet side-effect policy now blocks execution only; syntax, compile, and type-check validation still run for
  network, process, install, and server examples.

- Snippet discovery now ignores Alef-owned metadata files such as `.alef-snippet-coverage.json`.

- R documentation snippets now render idiomatic package calls, and C, Swift, Zig, and WASM snippets support visitor
  fixtures through the same native callback and call-preparation paths as their E2E tests.

- Cached E2E generation now reloads and reports the persisted fixture-snippet coverage ledger, so missing language
  cells remain visible instead of becoming false-green on an unchanged rerun.

- Targeted `alef generate --lang ...` cleanup now stays within the selected language's owned output roots instead of
  deleting generated files belonging to other targets.

- Python documentation snippets now retain their binding imports, and Java snippets declare typed JSON arguments while
  preserving explicitly qualified service class names.

- Documentation snippets now omit unused Go, TypeScript/WASM, C#, and Swift imports, keep Go imports formatter-ordered,
  and release native C results before exit.

- Swift optional-vector fields now use the JSON bridge instead of emitting `Option<Vec<T>>`, which swift-bridge
  0.1.59 cannot parse.

- Fixture extensions can now render protocol documentation recipes from typed AsyncAPI operations and WebSocket
  sessions. Documentation generation no longer inherits E2E harness skip directives.

- Node/WASM and Kotlin/Kotlin Android snippets now keep distinct target output directories while sharing their canonical
  TypeScript or Kotlin frontmatter and fence validation, preventing cross-target fixture path collisions.

- Targeted E2E orphan cleanup now requires a current artifact inside a language subdirectory before sweeping it, so
  top-level scaffolding and documentation snippets cannot delete an otherwise ungenerated test suite.

- Qualified field paths now restore the surviving public type name after a same-name internal type is excluded, avoiding
  lossy `String` sanitization for optional public configuration fields.

- Ruby, PHP, Elixir, and Dart documentation snippets now render standalone HTTP requests for HTTP harness fixtures,
  using the same normalized request bodies, content types, headers, and cookies as E2E generation.

- Missing Dart prebuilt libraries are now a quiet no-op during ordinary development builds, while package creation
  retains an actionable warning that published consumers will need a local native build.

- Emit runnable Python and Rust documentation entry points and native expected-error handling for Node and PHP snippets.

- External DTO roots now preserve qualified type identity when names overlap with native API types, including field
  references and qualified field exclusions, without false unmatched-exclusion warnings or forced field removal.

- Generate standalone C and Zig documentation programs and render native expected-error handling for C, Swift, and Zig.

- Targeted E2E generation now derives orphan-sweep roots exclusively from current E2E artifacts, preventing
  snippet-only output from deleting valid language test suites.

- Fix generated documentation snippets for Go, Dart, Java, Kotlin, C#, and PHP to include standalone runtime wrappers,
  required imports, and non-test error handling.

- Generated snippet paths, fences, and frontmatter now share the validator's canonical documentation language identity,
  including TypeScript for Node and WASM and Kotlin for Kotlin Android, while coverage retains the configured target.

- Crate-level validation suppressions now downgrade configured lossy surface, unknown type, ambiguous JSON value, and
  backend stub path diagnostics during extraction and generation while unsupported public generics remain fatal.

- E2E fixture schema validation now validates each element of top-level JSON arrays independently, accepts numeric
  identifier prefixes and project-specific fixture payloads, and reports the failing fixture index.

- Brew and Homebrew fixture snippet targets now use shell documentation metadata and report unsupported recipes through
  exact per-language coverage exceptions instead of aborting snippet generation with a language-mapping error.

- Multipart fixture requests now share one request plan across the generic and Rust e2e clients. Schema-only
  uploads synthesize a real multipart body and boundary, while explicitly empty form data emits neither.

- Snippet audits now count README-configured snippet paths, including language redirects, and fixture side-effect
  metadata uses the canonical safe, network, process, install, and server taxonomy without collapsing mutations.

- Validate configured documentation snippets in binding-aware per-language sessions so compile and run checks resolve
  local generated packages and manifests, with sanitized one-time setup commands and explicit preparation failures.

- Every configured fixture-language pair, including extension-backed rendering, now participates in deterministic
  snippet coverage accounting. Missing documentation metadata, empty recipes, and incompatible renderers remain
  visible unless an exact user-facing documentation exception explains the difference.

- Lossy binding-to-core conversion now boxes named fields after converting their binding values.

- PHP optional struct setters now borrow wrapper values accepted by ext-php-rs and clone them into owned core fields.

- Java, Kotlin, Kotlin Android, and C# documentation snippets now reuse backend-native typed argument and
  setup generation while preserving client factories, coroutine or async calls, and imports without test harnesses.

- PHP, Ruby, Elixir, and Dart documentation snippets now reuse their backend-native argument,
  setup, client, visitor, and streaming call preparation without emitting test assertions or teardown.

- Node, WASM, and Go documentation snippets now reuse backend argument builders for typed setup,
  imports, client factories, async calls, and binding-native function names without test assertions or harness code.

- Python and Rust documentation snippets now reuse their backend test renderers, preserving typed options,
  enum values, optional arguments, client factories, JSON request objects, async calls, and mock-server setup.

- Generated Python snippets now preserve top-level indentation and import boundaries, retain
  synthetic optional mock-server URL arguments, and fail syntax validation instead of treating
  indentation/parser errors as missing dependencies.

- Render fixture-driven documentation snippets with setup calls, imports, recipe-aware options and enum constructors,
  omitted absent optionals, client factories, mock-server URLs, Python handle constructors, valid Python/Ruby/Rust JSON
  literals, Rust async calls, side-effect frontmatter, whole-input arguments, and Rust/C language aliases.

### Added

- **The CLI can compare handwritten snippets with fixture-generated equivalents without writing files.**
  `alef e2e snippets-migrate <existing-root>` reports identical, different, and unmatched files in stable text or JSON.

- **Fixture snippet generation can target an explicit subset of E2E languages.** Set
  `[crates.e2e.snippets].languages` to stage generated documentation alongside languages that
  remain handwritten; an empty list continues to inherit the E2E target list.

- **Swift, Zig, and C/FFI documentation snippets now reuse their typed e2e call rendering.** Generated examples
  preserve backend imports, argument setup, allocator and environment handling while omitting test assertions and
  teardown, and reject complex harness-only patterns with contextual errors.

- **E2E backends now own documentation snippet bodies.** Snippet orchestration resolves the registered
  language generator, passes the extracted type and enum registries through, wraps backend output in
  shared Markdown metadata, and reports unsupported or unknown languages explicitly.

- **Snippet checks now produce versioned, source-aware reports and enforce explicit validation policy.**
  Results distinguish requested from effective validation levels, report downgraded probes, classify
  side effects, sanitize validator environments, reuse a persistent content-hash cache for changed-only
  checks, parse MDX frontmatter, and fail on empty discovery or report write errors. (`src/snippets`,
  `src/cli/commands/snippets.rs`)

- **Strict snippet checks now fail when any discovered example is skipped.** This prevents a
  configured validation gate from succeeding with zero validated snippets. (`src/cli/commands/snippets.rs`)

- **Generated snippet imports, setup statements, and calls now retain required line breaks.**
  (`src/e2e/templates/snippets/call.jinja`)

- **E2E fixtures can now generate deterministic, tested documentation snippets.** Optional fixture
  documentation metadata, declarative capability requirements, safe collision-checked output under
  `[crates.e2e.snippets]`, and migration comparison APIs let projects replace handwritten examples
  incrementally while preserving existing e2e output when snippet generation is not configured.
  (`src/e2e/snippets`, `src/core/config/e2e`)

- **Snippet validation can be configured and enforced across a generated workspace.** Docs snippet
  configuration now distinguishes reusable snippet roots from handwritten pages containing inline
  fences, supports exclusions, strict coverage and side-effect policies, and configures cache and
  report paths. Newly scaffolded Poly configuration runs the strict aggregate snippet check when
  snippet inputs are present. (`src/core/config/output`, `src/docs`, `src/scaffold/languages/poly.rs`)

## [0.59.0] - 2026-08-09

### Added

- **The HTTP e2e fixture model now carries every middleware category a fixture can declare.**
  `HttpMiddleware` gained `lifecycle_hooks`, `openrpc`, `background_tasks`, `websocket`, and
  `authorization`, and the struct is now `deny_unknown_fields`. Previously an undeclared category
  was silently discarded at parse time, so no generator in any language could ever see it; an
  unmodelled category is now a hard parse error rather than an invisible omission.
  (`src/e2e/fixture.rs`)
- **Generated Rust HTTP e2e tests assert the response body and headers, not only the status code.**
  Header checks skip values the transport or a response-encoding layer computes for itself.
  (`src/e2e/codegen/rust/http.rs`)

### Fixed

- **Generated Rust HTTP e2e tests sent string request bodies wrapped in quotes.** A string body was
  emitted through `serde_json::to_string`, so the payload reached the server with a leading and
  trailing `"`. Form-urlencoded bodies gained two characters that shifted every field index,
  multipart bodies received the two-character sequence `\\r\\n` instead of CRLF, and deliberately
  malformed JSON payloads arrived as valid JSON strings. String bodies are now emitted verbatim;
  structured bodies are unchanged. (`src/e2e/codegen/rust/http.rs`)

## [0.58.3] - 2026-08-09

### Fixed

- **Linux CLI release archives now build on available GitHub-hosted x86_64 and arm64 runners.**
  (`.github/workflows/publish.yaml`)
- **The publish workflow now uses GitHub-hosted runners for release orchestration jobs.** This
  removes the unavailable `runner-medium` dependency from preparation, validation, release checks,
  asset upload, package publishing, and finalization so published releases can produce their
  downloadable CLI archives. (`.github/workflows/publish.yaml`)
- **TypeScript e2e nested-type discovery is deterministic when distinct Rust types share a short
  name.** Candidate resolution now uses the full Rust path as a stable tie-breaker, preventing
  generated WASM tests from changing with input order. (`src/e2e/codegen/typescript/test_file`)
- **PHP bindings now generate working setters for optional named-struct fields.** Setter signatures,
  native conversions, and generated type stubs consistently accept nullable wrapped structs instead
  of dropping or mis-typing the assignment path. (`src/backends/php/gen_bindings`)
- **PyO3 trait bridges now preserve mutable callback updates and deserialize unit-enum returns correctly.**
  Async callbacks with `&mut` named parameters write an optional host-returned replacement back to
  Rust, protocol stubs expose that contract, and unit-only enums accept natural bare variant names
  without weakening struct-return validation. (`src/backends/pyo3`)
- **Snippet audits and coverage reports now recognize Astro MDX imports and `.mdx` documentation files.**
  MDX `Content` imports resolve relative to the importing page, and both audit and gap detection
  include `.mdx` alongside Markdown when checking references and fenced languages.
  (`src/snippets/audit.rs`, `src/snippets/gaps.rs`)
- **Generated Elixir e2e suites could leave the harness running as an orphan, hanging `mix test`
  after every test had already passed.** `test_helper.exs` spawned the harness with
  `Port.open({:spawn_executable, ...})` and never reaped it. Closing an Erlang port only closes the
  child's stdin; the harness runs `elixir -noshell` and never reads stdin, so it survives the port
  close, gets reparented to init, and keeps the stdout pipe it inherited from the test runner open —
  leaving the runner blocked on EOF indefinitely. The template now captures the harness's OS pid via
  `:erlang.port_info(port, :os_pid)` and reaps it in `ExUnit.after_suite`, `kill`-ing it and falling
  back to `kill -9` after a grace period. Only a harness this process spawned is touched — the
  existing `SUT_URL` guard still leaves an externally supplied harness untouched.
  (`src/e2e/templates/elixir/test_helper_server.exs.jinja`)

## [0.58.2] - 2026-08-09

### Fixed

- **Generated Swift e2e suites could leave the harness running as an orphan, hanging `swift test` and
  corrupting later runs.** `setUp` piped the harness's `standardOutput` without ever draining it — an
  undrained pipe blocks the child once the kernel buffer fills — and never assigned `standardError`
  at all, so the child inherited the test runner's stderr descriptor and could keep it open
  indefinitely. There was no `tearDown` anywhere, so nothing reaped the process. Both streams now go
  to `FileHandle.nullDevice`, and a new `tearDown` terminates and waits on the harness it spawned.
  Because `swift test` runs every class in one process and `setUp` only spawns when `SUT_URL` is
  unset, the spawning class also clears `SUT_URL` on teardown so the next class spawns its own
  harness rather than addressing the one just killed; an externally supplied `SUT_URL` is left
  untouched. An orphan surviving on the fixed port also silently redirected *other* languages' e2e
  suites at the wrong server, since every generated suite probes the port without verifying
  ownership. (`src/e2e/codegen/swift/test_file.rs`)

### Changed

- **The Rustler backend selected its handler-wrapper template by matching the registration method
  name against the literal `"route"`**, a product-specific string in generator core. It now
  dispatches structurally on the existing `HandlerShape` IR enum, emitting the context-object wrapper
  only for `HandlerShape::ContextObject`. That field existed but was never populated from
  configuration; `[[crates.services.registrations]]` entries now accept `handler_shape`
  (`"bare_callable"` — the default — `"context_object"`, `"request_response"` or
  `"introspect_params"`), resolved in the service extractor. Consumers relying on the old behaviour
  must set `handler_shape = "context_object"` on the affected registration. The wrapper template was
  previously unreachable in tests, because the fixture's registration was named `add_handler` and so
  never satisfied the name gate; it is now covered by a positive and a negative case.
  (`src/backends/rustler/gen_bindings/service_api`, `src/core/config/service.rs`,
  `src/extract/extractor/service.rs`)

## [0.58.1] - 2026-08-08

### Fixed

- **The Rustler backend generated an Elixir binding in which every request reaching a user handler
  hung forever.** Three defects compounded. (1) Chainable opaque wrapper methods returned the bare
  NIF `reference()` instead of re-wrapping in `%__MODULE__{ref: ...}`, because `returns_self` was
  computed as `is_static && returns_self` — excluding every receiver-based (`&mut self -> Self`)
  builder method, which is the only kind a builder chain uses. A builder therefore degraded from
  struct to bare reference on the first chained call. (2) An opaque metadata parameter was emitted
  bare into the registration tuple, passing the Elixir wrapper struct where the NIF decodes
  `rustler::ResourceArc<T>`. (3) The handler `GenServer` received on `handle_cast/2`, but the Rust
  bridge dispatches with a raw `send/2`, so `{:trait_call, ...}` fell through to the default
  `handle_info/2` and was silently discarded. Raw-send + `handle_info` is already the contract used
  by the scaffold, the e2e stubs and the Gleam trait bridge, so the two service-API templates were
  the outliers. (`src/backends/rustler`)
- **The Elixir e2e HTTP client JSON-encoded raw request bodies.** `render_call` unconditionally used
  Req's `json:` option, so a pre-encoded form or multipart payload was sent as a quoted JSON string
  and rejected by the server; multipart calls additionally never emitted a `Content-Type` header at
  all, since `ctx.content_type` was consulted only for the body decision. It now sends raw bodies via
  `body:` and falls back to `ctx.content_type` for the header. The content-type resolution added for
  Java in 0.58.0 is now a shared helper (`effective_content_type` / `is_raw_text_content_type` in
  `src/e2e/codegen/client`) rather than a third inline copy. (`src/e2e/codegen/elixir/http.rs`)
- **The Elixir e2e client silently dropped either request headers or cookies** when a fixture had
  both, by emitting two separate `headers:` options in one keyword list. They are now merged into a
  single option. (`src/e2e/codegen/elixir/http.rs`)
- **Generated Dart and WASM test files reordered their imports between otherwise identical runs.**
  Trait-import collection used a `HashSet`, and the transitive WASM nested-type walk returned a
  field-name-keyed `HashMap` in which two classes sharing a field name collided — which one survived
  depended on iteration order. Both now use `BTreeSet`, and the WASM walk returns a set of class
  names, which is all its only consumer needs. Generated `e2e/` output is byte-compared by CI, so
  this ordering must be deterministic. (`src/e2e/codegen/dart`, `src/e2e/codegen/typescript`)

### Added

- **`skip.languages` ids in fixtures are validated against the configured e2e target list.** An id
  that matched no real target silently disabled nothing, so the fixture kept running everywhere the
  author believed it was skipped. (`src/e2e/fixture.rs`)

## [0.58.0] - 2026-08-08

### Added

- **Kotlin value types bridge their instance methods through JNI shims**, so methods declared on a
  value type are now callable from Kotlin rather than being dropped at the binding boundary.
  (`src/backends/kotlin`)
- **Dart emits the FRB cfg-gate carry helper into `build.rs`**, carrying `cfg` gates through the
  flutter_rust_bridge codegen so gated items compile consistently. (`src/backends/dart`)

### Fixed

- **The Java e2e HTTP client now percent-encodes reserved characters in an embedded query and
  honours a form Content-Type declared only in a request header.** `java.net.URI.create` is
  RFC-2396-strict, so a fixture whose `request.path` embedded a raw query such as
  `?tags=a|b|c` threw `IllegalArgumentException` (lenient clients like Python and Node accept it).
  Separately, a fixture that declared `application/x-www-form-urlencoded` only in `request.headers`
  (leaving the request `content_type` field unset) had its string body JSON-encoded — the quoted
  body was then rejected by the server. The renderer now sanitizes the query segment and consults
  the header when deciding whether to send a raw body. (`src/e2e/codegen/java/http.rs`)
- **Zig e2e assertions stay on the raw JSON navigation path** instead of diverging onto a typed path
  that did not match the generated harness. (`src/e2e/codegen/zig`)
- **WebAssembly generation emits compilable conversions for delegating and payload-enum types.**
  (`src/backends/wasm`)

### Changed

- Updated the `jsonschema` crate to 0.49.7.

## [0.57.1] - 2026-08-07

### Fixed

- **The Dart module file no longer emits an unused `import 'traits.dart';`.** 0.56.0 added the
  import unconditionally so that a doc comment naming a trait (`[OcrBackend]`) would not trip
  `comment_references`. But the module file usually names no trait at all, so `dart analyze` then
  reported the import as `unused_import` — a hard lint failure in every consuming repo that
  regenerated on 0.56.0 or 0.57.0. The import is now emitted only when the generated body actually
  refers to one of the configured bridge trait names, which keeps both lints satisfied.
  (`src/backends/dart/gen_bindings/mod.rs`)

## [0.57.0] - 2026-08-07

### Changed

- **MSRV raised to 1.88.** The declared 1.85 floor was never real: `zip` 8.6 requires 1.88 and
  `criterion` 0.8.2 requires 1.86. Because `cargo upgrade` is MSRV-aware, the false floor made it
  propose *downgrades* (`libloading` 0.9→0.8, `zip` 8→7, `criterion` 0.8→0.7) instead of upgrades,
  so dependency maintenance had to route around it with `--ignore-rust-version`. Raising the floor
  also unlocks clippy's let-chain `collapsible_if` suggestions, applied across 868 sites in 250
  files. Consumers building alef from source now need Rust 1.88 or newer. (`Cargo.toml`)

### Fixed

- **Generated Elixir e2e/test_apps projects are now formatted by `mix format`.** `.ex`/`.exs` are
  excluded from poly's pass so `mix format` can own them, but `mix format` only ever ran in
  `packages/elixir` — the generated e2e and test_apps suites were therefore formatted by nothing at
  all and shipped exactly as the emitter wrote them, with calls left unwrapped well past the line
  limit. A `.formatter.exs` is now emitted next to the generated `mix.exs` (a bare `mix format` has
  no `inputs:` without one, so it refuses to run), and `mix format` runs over the directory as an
  Elixir residual alongside the existing `go mod tidy` one. `line_length` matches the binding
  package's `.formatter.exs` so every generated Elixir tree wraps identically; `import_deps` is
  deliberately omitted so formatting never depends on a fetched `deps/`.
  (`src/e2e/codegen/elixir.rs`, `src/e2e/format.rs`)

- Two redundant derefs in the PHP type-stub backend that were failing `poly lint` on main.
  (`src/backends/php/gen_bindings/type_stubs.rs`)

## [0.56.0] - 2026-08-07

### Changed

- **BREAKING: `FieldDef` gains a `version` field.** `alef(since = "...")` written on a struct
  field was parsed and immediately discarded — every other IR item (structs, methods, params)
  already carried a `VersionAnnotation`, but the field-level annotation had nowhere to land.
  `FieldDef` now has `pub version: VersionAnnotation` alongside the rest, so field-level `since`/
  `deprecated` metadata survives extraction and reaches backends. Any code that builds a `FieldDef`
  with an exhaustive struct literal — the pattern used ~300 times in this crate's own extractor and
  IR-construction tests, and likely used by any downstream code that constructs the IR directly
  rather than only reading it — now fails to compile with a missing-field error. Add
  `..Default::default()` to the literal (the field carries `#[serde(default)]`, so deserializing an
  older IR document is unaffected) or set `version` explicitly if you need to preserve field-level
  annotations. (`src/core/ir/items.rs`)

### Fixed

- **A boxed field on a struct-variant (named-field) enum arm now converts correctly in both
  directions.** wasm tagged-enum codegen already threaded `field.is_boxed` through the tuple-variant
  branch, but the named-field branch ignored it, so a `Box<T>` payload on a struct variant generated
  a conversion with no `Box::new`/deref — code that did not compile. Both branches now share
  `box_wrap_map_into`/`box_unwrap_map_into`/`box_unwrap_into` helpers, so tuple and struct variants
  wrap and unwrap boxed fields identically in both directions.
  (`src/backends/wasm/gen_bindings/enums.rs`)

- **A wasm type that drops a field during extraction no longer emits a delegating `Default` impl it
  cannot satisfy.** The delegating impl is `<core::T as Default>::default().into()`, which requires
  a `From<core::T>` able to carry every core field into the binding type; a field omitted from the
  binding (e.g. an unknown/sanitized type) makes that conversion impossible to generate correctly.
  Such types now fall back to `#[derive(Default)]` on the fields the binding actually has, matching
  the same core-to-binding convertibility check already used for the `From` impl itself.
  (`src/backends/wasm/gen_bindings/mod.rs`)

- **Generated Dart code passes `dart analyze` with zero warnings.** Three independent issues: `lib.dart`
  exported `traits.dart` but never imported it, so every `///` doc reference to a plugin trait was an
  unresolvable `comment_references` — `export` puts a name in downstream scope, not the exporting
  file's own scope, and doc-comment resolution only looks at the latter; `render_type` unconditionally
  added `import 'dart:typed_data'` even when the FRB typed-list import already superseded it, tripping
  `unused_import`; and the scaffolded `bin/download_libs.dart` reached into `lib/` via a relative
  `../lib/...` path instead of the package import. `lib.dart` now also imports `traits.dart`, the
  redundant `dart:typed_data` import is dropped once the FRB import is present, and
  `download_libs.dart` uses a `package:` import.
  (`src/backends/dart/gen_bindings/mod.rs`)

- **flutter_rust_bridge no longer emits calls to functions that were compiled out.** FRB is not
  feature-aware: it generates bindings straight from `lib.rs`, so a function behind a `#[cfg(...)]`
  gate that a reduced feature set (e.g. Android's trimmed OCR backend list) compiles out still gets a
  generated call site, which fails to build. A new post-build step,
  `PostBuildStep::CarryFrbCfgGates`, reads the `#[cfg(...)]` gates directly off `lib.rs` and rewrites
  the frb-generated glue to carry the same gates, via `carry_lib_rs_cfg_gates_into_frb_generated`.
  (`src/backends/dart/frb_rewrite/cfg_gates.rs`, `src/backends/dart/frb_rewrite.rs`,
  `src/cli/pipeline/commands/build.rs`)

- **The generated PHPStan stub declares a getter for every binding field, matching the extension it
  describes.** The real ext-php-rs extension emits a getter for every field unconditionally (a
  `for field in binding_fields(&typ.fields)` loop in `structs.rs`), including fields with no
  constructor-param support. The stub used to skip some of those, so PHPStan reported a false
  "undefined method" on a getter call that works fine at runtime. The stub's getter loop now mirrors
  the extension's exactly, including the `?string` return type Json/untagged-enum getters always
  serialize to regardless of the field's own optionality.
  (`src/backends/php/gen_bindings/type_stubs.rs`)

- **Generated Python stubs annotate `Json` fields as `str`, not `dict[str, Any]`.** `Pyo3Mapper::json()`
  maps `TypeRef::Json` to Rust `String`, so the field is always a JSON-encoded string at the pyo3
  boundary — the stub previously advertised `dict[str, Any]`, a type the runtime value never actually
  has. Stubs now declare `str`, and the now-unneeded `from typing import Any` import tied to that
  annotation is dropped so it doesn't trip ruff's `F401` (the `from_native` converters are still the
  only remaining source of `Any`). (`src/backends/pyo3/gen_bindings/types.rs`)

## [0.55.8] - 2026-08-07

### Fixed

- **`serde` attributes hidden behind `cfg_attr` are honoured again, so enum wire names under a
  conditional `rename_all` are correct.** `extract_serde_rename_all` unwrapped `cfg_attr` with
  `Attribute::parse_nested_meta`, which silently gave up when the condition was anything more
  complex than a bare ident or `feature = "x"` — so a `#[cfg_attr(any(feature = "serde", feature =
  "metadata"), serde(rename_all = "snake_case"))]` enum was extracted as having no `rename_all` at
  all. That was harmless until 0.55.7 changed the Java backend's no-`rename_all` fallback from
  lowercasing the variant to emitting it verbatim: the two agree for single-word variants
  (`Auto` → `auto`), so the missing attribute only became visible once the fallback changed, at
  which point the generated Java sent `Auto` to a core that deserialises `auto` and every call
  failed with ``unknown variant `Auto` ``. The condition is now parsed structurally as a
  `syn::Meta` (handling `any`/`all`/`not` and nesting), nested `cfg_attr` is unwrapped
  recursively, and the bare and `cfg_attr` paths share one walk. The predicate itself is still
  never evaluated — alef cannot know which features a downstream build enables, so every inner
  attribute is treated as if it applied unconditionally.
  (`src/extract/extractor/helpers/attributes.rs`)

## [0.55.7] - 2026-08-07

### Added

- **The Swift bridge crate's injected FFI dependency accepts per-target overrides.** A new
  `[crates.swift] ffi_target_dep_overrides` list — `cfg`/`features`/`default_features`, the same
  shape as `target_dep_overrides` — moves the secondary `*-ffi` dep out of the flat `[dependencies]`
  table into one `[target.'cfg(...)'.dependencies]` block per predicate, with the default gated on
  `cfg(not(any(...)))`. Until now `ffi_features` could only apply to a single ungated dep line, and
  because Cargo unifies features across every edge to a package, an unconditional `full-no-heic`
  pulled `sceptre-ocr-ort` onto iOS even where the core dep asked only for `android-target`,
  tripping the mobile `compile_error!` guards; xberg carried a hand-written post-regen patch that
  every local `alef all` reverted (#370). The FFI and core target entries are merged into one
  globally sorted list, since cargo-sort orders all target tables per manifest, not per dependency.
  Empty by default, so a config that sets only `ffi_features` is byte-identical.
  (`src/core/config/languages/swift.rs`, `src/backends/swift/gen_rust_crate/cargo.rs`,
  `src/backends/swift/gen_rust_crate/mod.rs`)

### Fixed

- **Go e2e harness import no longer collides with a reserved keyword.** The harness derived its
  import alias from the last segment of the module path with no sanitization, so a module ending in
  `/go` (e.g. `.../packages/go`) emitted `import go "..."` — and `go` is a reserved word, so the file
  failed to compile with `missing import path`. Aliases are now routed through a new `go_ident`
  helper that escapes reserved keywords and invalid identifiers (`go` → `go_`).
  (`src/core/keywords.rs`, `src/e2e/codegen/go.rs`)

- **`alef update`/`upgrade` no longer corrupt a pnpm project's `package.json`.** The default Node
  recipes ran bare `pnpm up -r` (and `pnpm up --latest -r -w`). With pnpm's default
  `auto-install-peers`/`dedupe-peer-dependents`, `pnpm up` promotes the optional peer deps of
  installed packages (e.g. napi-rs's `@emnapi/core`, `@emnapi/runtime`, `@octokit/core`, `typanion`)
  into the project's own `dependencies` and stamps them with the *workspace* version — so every
  update rewrote `package.json` with bogus, version-mismatched dependencies. Both recipes now pass
  `--config.auto-install-peers=false --config.dedupe-peer-dependents=false`, so only the real,
  declared dependency ranges are bumped. (`src/core/config/update_defaults.rs`)

- **PHP streaming methods are emitted in adapter-declaration order.** The PHP backend collected the
  streaming method keys into an `AHashSet` and then *iterated* it to emit the `#[php_impl]` methods —
  the only place in the backend where a hash container drove output order. ahash seeds itself per
  process, so regenerating an unchanged tree could swap two streaming methods in the generated Rust
  binding, producing a spurious diff and an intermittently red `alef verify` freshness gate. The keys
  are now an order-preserving, deduplicated `Vec` built from `config.adapters`, matching the
  config-declared order every other PHP emitter already uses.
  (`src/backends/php/gen_bindings/rust_bindings.rs`,
  `src/backends/php/gen_bindings/types/structs.rs`)

- **The scaffolded PHP `composer.json` declares a PHPUnit constraint that is installable on the PHP
  version it claims to support.** The generated manifests paired `"php": ">=8.2"` with
  `"phpunit/phpunit": "^13.1"`, but PHPUnit 13 requires PHP >= 8.4.1 — so `composer install` could
  not resolve on 8.2 or 8.3, and Dependabot, which resolves Composer against the declared platform
  floor rather than the runtime PHP, failed on every run in the consumer repos. The constraint is now
  `^11.5 || ^12.0 || ^13.1`, letting Composer pick the newest major the actual PHP supports.
  (`src/core/template_versions.rs`)

- **Java enum wire names now match serde's actual no-`rename_all` fallback.** The Java backend's
  tagged-discriminator and simple-enum generators lowercased the variant name (`listitem`) when an
  enum had no `#[serde(rename_all)]`, but serde with no rename attributes emits the PascalCase
  variant name verbatim (`ListItem`). Generated `json_name` values — and the matching
  `excluded_variants` handling — now fall through to the same verbatim behavior as every other
  backend. (`src/backends/java/gen_bindings/types/enums.rs`)

- **NAPI tagged-enum discriminator wire names now match the declared `#[serde(tag = ...)]`
  contract.** (#218, @thisislvca)

- **NAPI tagged-enum sanitized fields no longer drop data or emit non-compiling conversions.**
  #218 (@thisislvca) fixed the tagged-enum discriminator wire names but its sanitized-field
  handling had follow-on gaps: an unreachable `optional` branch inside the `sanitized` arm meant
  `field_conversion_from_core` was always called with `optional: false`; checking `f.optional`
  before `f.sanitized` meant an `Option<Vec<(String, String)>>` field never reached the sanitized
  path in either direction; gating on any `Vec<_>` shape (rather than the specific
  `Vec<Vec<String>>` shape actually handled) could emit a `format!("{:?}", …)` assigned to a
  `Vec<_>`-typed field, which does not compile; and the core→binding direction re-parsed a
  rendered `"name: expr"` string with `strip_prefix`/`replace` instead of composing an expression
  directly. Sanitized `Vec<Vec<String>>` (optional and non-optional) and `Map<String, String>`
  fields now convert correctly in both directions; every other sanitized shape keeps the
  pre-#218 `Default::default()` / `None` fallback, which always compiles.
  (`src/backends/napi/gen_bindings/methods.rs`,
  `src/codegen/conversions/helpers/field_fragments.rs`)

- **Generated `[target.'cfg(...)'.dependencies]` tables are ordered the way `cargo-sort` expects.**
  cargo-sort enforces table order, not just entries within a table: target-cfg blocks sort
  alphabetically by the raw cfg predicate, byte-wise. Every generator emitted the default
  `cfg(not(any(...)))` branch first, which is only coincidentally correct — `not(` sorts after
  `all(` but before `target_os`, so an `all(...)` override (xberg's macOS-Intel target) produced an
  unsorted manifest that `cargo sort --check`, and hence `poly lint`, rejects. A new
  `join_sorted_target_dep_blocks` sorts the default branch together with every override, and the
  FFI, JNI, Dart and shared `render_core_dep_with_overrides` (python/node/ruby/php/elixir) emitters
  all route through it. Separately, the wasm template emitted `[dev-dependencies]` ahead of its
  trailing `getrandom` target block; it now comes after. (`src/scaffold/mod.rs`,
  `src/scaffold/languages/ffi.rs`, `src/scaffold/languages/jni.rs`,
  `src/backends/dart/gen_rust_crate/cargo.rs`, `src/backends/wasm/gen_bindings/cargo.rs`)

- **The Swift bridge crate's `Cargo.toml` emits `[build-dependencies]` before `[lints.rust]`.** The
  manifest format string placed the lints table between the target-cfg blocks and
  `[build-dependencies]`, which is not the section order `cargo-sort` accepts, so the generated
  manifest failed `cargo sort --check`. (`src/backends/swift/gen_rust_crate/cargo.rs`)

- **`sync-versions` leaves unpublished manifests at their own version.** The release version was
  stamped onto every manifest the pipeline globbed, including `publish = false` workspace members —
  compatibility shims that exist only to keep a path dependency resolvable — and npm `package.json`
  files marked `"private": true`. Neither is ever published, so the churn was pure noise in every
  release diff. `publish` is now parsed properly by `manifest_is_publishable`: absent, `true` and
  `["some-registry"]` all stay publishable and only the literal `false` is skipped. Both that check
  and the new `package_json_is_private` fail open — a missing or unparseable manifest counts as
  publishable — so an odd manifest shape cannot silently freeze a real crate's version.
  (`src/publish/workspace.rs`, `src/cli/pipeline/version_workspace.rs`,
  `src/cli/pipeline/version_core.rs`, `src/cli/pipeline/version.rs`)

- **The PHP, NAPI and wasm emitters use field-init shorthand instead of a redundant `x: x`.** All
  three built struct literals with an unconditional `format!("{}: {}", name, expr)`, so any field
  whose expression is just its own name came out as a `clippy::redundant_field_names` violation —
  which is why xberg's generated crates carry a file-level allow for it: 423 sites in php, 318 in
  wasm, 18 in node. Each emitter now compares the field name against the expression and emits the
  bare name when they are equal, porting the guard the PyO3 backend already had (and why its count
  is zero). A field whose type genuinely needs a cast or wrap keeps its full `field: expr` form.
  (`src/backends/php/gen_bindings/types/structs.rs`, `src/backends/napi/gen_bindings/methods.rs`,
  `src/backends/wasm/gen_bindings/types.rs`)

- **Nested `Json` maps to `JsonElement` at any depth in the generated C# DTOs.**
  `csharp_type_for_dto_field` matched only bare `Json`, `Map<_, Json>` and `Option<Json>` and then
  fell through to `csharp_type`, which maps `Json` to `string` — so `Vec<Value>` became
  `List<string>`, reintroducing the exact "Cannot get the value of a token type 'StartObject' as a
  string" failure the function's own doc comment says it exists to prevent. It now recurses through
  `Optional`, `Vec` and `Map`, reusing the same wrapping formats as `CsharpMapper`'s
  `optional`/`vec`/`map` combinators, so non-Json types still resolve exactly as `csharp_type` does.
  The Java `resolve_field_type` doc comment is corrected in the same pass: it claimed unknown
  `Named` types are replaced with `JsonNode` when the backend actually emits `Object`, so the doc
  was wrong, not the code. (`src/backends/csharp/type_map.rs`,
  `src/backends/java/gen_bindings/types/shared.rs`)

- **Generated `From` impls carry only the clippy allows they can actually trigger.** Every emitted
  impl had an unconditional `#[allow(clippy::redundant_closure, clippy::useless_conversion)]` —
  ~1435 sites across xberg's four generated crates, half of it duplicating a crate-level allow. A
  new `needs_clippy_allow` scans the assembled field/statement/argument fragments for `(|` and for
  `.into()`/`Into::into`, and each allow is emitted only when its lint can fire, matching how
  `needless_update` was already gated. Two of the underlying closures are removed rather than
  suppressed: an optional `Arc` core wrapper now emits `.map(std::sync::Arc::new)` instead of
  `.map(|v| std::sync::Arc::new(v))`, and a newtype over an identity/tuple-passthrough `Named` type
  drops the no-op `.into()`. (`src/codegen/conversions/helpers/clippy_allow.rs`,
  `src/codegen/conversions/binding_to_core/render.rs`,
  `src/codegen/conversions/core_to_binding/render.rs`,
  `src/codegen/conversions/binding_to_core/wrappers.rs`,
  `src/codegen/templates/conversions/binding_to_core_impl.jinja`,
  `src/codegen/templates/conversions/core_to_binding_impl.jinja`)

- **A boxed opaque field converts to `Box<T>` in both directions.** `field.is_boxed` was ignored on
  both opaque paths. Binding→core moved the opaque wrapper's `Arc<T>` handle out of `.inner`
  directly, overwriting the `Box::new` applied upstream and yielding `Option<Arc<T>>` where the core
  struct declares `Option<Box<T>>`; core→binding nested the value as `Arc<Box<T>>`. Both branches
  now deref-clone the shared value and rebox it. The core→binding unbox rewrite also matched its
  input by exact string equality against `val.<field>.map(Into::into)`, so any other producer
  silently skipped the deref; it is structural now — strip the `<field>: val.<field>` prefix, unbox,
  then re-apply whatever the rest of the expression already did. Boxed struct fields had no test
  coverage in either direction. (`src/codegen/conversions/binding_to_core/render.rs`,
  `src/codegen/conversions/core_to_binding/render.rs`)

- **`mix format` is the sole formatter for generated Elixir; poly no longer touches `.ex`/`.exs`.**
  poly's pure-Rust Elixir formatter rewrites valid, mix-compliant source — `|>` pipe continuation
  drops from 6 spaces to 4, multi-line struct/map field continuation collapses to flush-left — and
  then its own `--check` reports the corrupted result as clean, so no freshness gate caught it: 247
  generated Elixir files in xberg drifted with every gate passing. Fixing the templates could not
  work, because poly re-corrupted correctly indented input on every run. Both the `--fix` and
  `--check` poly invocations now pass `--exclude **/*.ex --exclude **/*.exs`, and `mix deps.get`
  followed by `mix format` runs as an Elixir residual on a partial regen and once after the
  full-regen convergence loop. `mix` joins `required_formatters` whenever Elixir is targeted, so a
  missing binary warns loudly rather than silently leaving the output unformatted — silent skipping
  is what let this hide in the first place. (`src/cli/pipeline/format.rs`)

## [0.55.6] - 2026-08-06

### Fixed

- **The Dart native loader downloads and caches the library again on a cold cache.** alef had
  two divergent implementations of the same injected `_alefResolveExternalLibrary` prologue: a
  hardcoded `format!` in `frb_rewrite::external_library_loader` and
  `dart_init_prologue_replacement.jinja`, rendered into the generated bridge crate's `build.rs`.
  Within alef's own pipeline the `format!` variant always wins — a `post_build` FRB regeneration
  clobbers `build.rs`'s patch before `FrbDartSealedVariants` runs — and that variant only ever
  *read* the versioned cache, so a cache miss threw `StateError` even though
  `nativeDownloadAndCacheLibrary()` was defined and exported for exactly that case. Both call
  sites now render the one template, which keeps the `format!` variant's improvements
  (absolute-path `dlopen`, the `Platform.script` package-root fallback, the descriptive miss) and
  restores the download-on-miss step ahead of the `StateError`.
  (`src/backends/dart/templates/dart_init_prologue_replacement.jinja`,
  `src/backends/dart/frb_rewrite/external_library_loader.rs`,
  `src/backends/dart/gen_rust_crate/cargo.rs`)

- **`build.rs`'s embedded loader searches for the library that is actually built.** The bridge
  crate emitted at `packages/dart/rust/` is `<source>-dart`, so its cdylib is
  `lib<source>_dart.dylib` — but the source crate name was passed as the candidate stem, leaving
  the embedded loader looking for a `libhtml_to_markdown_rs.dylib` that no build produces. Only
  reachable when a consumer builds the bridge crate outside alef's pipeline, where it silently
  degraded every bundled-native lookup into a cache lookup.
  (`src/backends/dart/gen_rust_crate/mod.rs`)

- **The loader's "not found" message now names the actual environment variable.** The override
  was suggested as an escaped `\$nativeLibDirEnv`, so Dart printed the identifier rather than
  interpolating it and the reader was told to set a variable whose name was never given. The
  lookup also repeated the variable's value as a string literal instead of reading the
  `nativeLibDirEnv` constant, leaving two places that had to agree on it.
  (`src/backends/dart/templates/dart_init_prologue_replacement.jinja`)

## [0.55.5] - 2026-08-06

### Fixed

- **The CLI release now includes a Windows binary.** The publish matrix built only
  linux-x86_64, linux-aarch64 and macos-arm64, while the archive step's `.zip` branch and
  its `disable-cache` toggle were already written for Windows — the matrix entry was simply
  missing. `xberg-io/actions/install-alef` therefore found no asset on a Windows runner and
  fell back to `cargo install --git --tag`, building alef from source on every Windows job that
  installs it: 441s, 550s and 651s in html-to-markdown's three Windows Python e2e jobs alone.
  (`.github/workflows/publish.yaml`)

## [0.55.4] - 2026-08-06

`v0.55.2` and `v0.55.3` were tagged and pushed but never published to crates.io — the
`Publish` workflow only triggers on `release: types: [published]`, and no GitHub release
was created for either tag (see the `publish-flow` fix below). Their fixes are folded into
this section, in the order they actually landed, since 0.55.4 is the first version anyone
could actually install.

### Fixed

- **`nativeFree<Owner>` calls now pascal-case an acronym owner in the generated Kotlin JNI
  client's `close()`.** `close()` built the free-function name from the class name verbatim
  (`nativeFreeGraphQLRouteConfig`), while every other JNI emission site pascal-cases the
  owner via `to_pascal_case` (`nativeFreeGraphQlRouteConfig`) — so `close()` on any client
  type whose name contained an acronym called a native function that was never registered.
  `close()` now derives `free_name` from `to_pascal_case(class_name)`, matching the bridge's
  `external fun` declaration and the Rust JNI export.
  (`src/backends/kotlin/gen_bindings/jni_emitter/client_class.rs`)

- **Generated FFI free functions compile under edition 2024.** `free_function_header.jinja`
  emitted `pub extern "C" fn ...` for the generated `_free` shims; edition 2024 requires an
  `extern "C"` function containing raw-pointer or FFI-unsafe operations to be written as
  `unsafe extern "C" fn`, so every generated FFI binding with a free shim failed to compile.
  The template now emits `pub unsafe extern "C" fn`.
  (`src/backends/ffi/templates/free_function_header.jinja`)

- **The generated `poly.toml` is now poly-canonical when it is written.** `toml_array` hard-coded a
  4-space indent while its doc-comment claimed to emit "taplo's canonical multi-line form" — taplo
  uses 2 — and several inline arrays carried inner padding (`select = [ "correctness", … ]`). The
  freshly written file therefore never matched the committed one, so the byte-equality skip in
  `write_scaffold_files_with_overwrite` never fired and `poly.toml` was rewritten on every run in
  every repo. What normally hid this is the post-generation `poly fmt --fix` pass repairing it
  afterwards — but that runs after post-build, stubs, README, e2e and docs, so an abort in any of
  those leaves the raw file behind (observed on xberg, where the run died in the Dart FRB
  post-build), and the partial-regen paths never pass the repo root to poly at all. The emitter now
  matches taplo, and `poly.toml` is handed to poly immediately after it is written rather than many
  fallible stages later.
  (`src/scaffold/languages/poly.rs`, `src/cli/pipeline/generate/scaffold.rs`)

### Added

- **`[tools.mix]` is emitted for repos with an Elixir binding.** poly has no native Elixir formatter
  and `tree-sitter-elixir` ships no `indents.scm`, so poly reindented `.ex`/`.exs` with a hand-rolled
  query that modelled only `do…end` and `fn…end`; every other construct was re-emitted at column 0
  and poly then fought `mix format` indefinitely. Declaring the catalog tool hands the language to
  `mix format` — poly ≥0.19.6 drops its own reindenter when a runnable catalog formatter owns the
  language. (`src/scaffold/languages/poly.rs`)

## [0.55.1] - 2026-08-05

### Fixed

- **Generated Rust e2e harness compiles under edition 2024.** `tests/common.rs` called
  `std::env::set_var` at three points to publish the mock-server URLs, which edition 2024 made an
  unsafe function, so every integration-test binary failed to build with `error[E0133]` and the
  whole Rust e2e suite was uncompilable (seen on liter-llm). The calls are now wrapped in `unsafe`
  with a SAFETY note: they run inside the `OnceLock` initializer, before any test thread exists.
  (`src/e2e/codegen/rust/mock_server/common_module.rs`)

## [0.55.0] - 2026-08-05

### Changed

- **Python: a field whose name matches a method is an attribute again, not a bound method.** When a
  core type declared both a public field and a same-named inherent method, the PyO3 backend emitted
  a `#[pyo3(get)]` getter *and* a `#[pymethods]` wrapper. The wrapper is registered last and kills
  the getter, so `config.providers` silently returned a bound method while the generated stub and
  the constructor keyword both promised a list. The method wrapper is now skipped and the attribute
  wins, matching every other binding. Any caller written against the accidental `config.providers()`
  spelling must drop the parentheses.

### Fixed

- **A field and a same-named method no longer collide in the Go, Ruby, Swift and C# backends.** The
  same defect already fixed for the FFI (0.54.1) and WASM (0.54.2) backends, in four more emitters
  an earlier survey wrongly cleared. Go emitted both into one struct (`field and method with the
  same name Providers` — a hard compile error); Ruby emitted a duplicate inherent method
  (`error[E0592]`), a duplicate `define_method`, and an RBS `DuplicatedMethodDefinition` that failed
  `steep`; Swift and C# admitted the collision but had no live instance downstream. Each backend now
  emits the field and skips the method. A parameterized method of the same name is still emitted.
- **alef's own CI is green again.** The e2e PHP composer tests hardcoded real downstream project
  names, which `check_project_mentions.py` forbids — alef must stay project-agnostic — failing
  `no_project_name_special_casing_in_enforced_files` on all three platforms.

## [0.54.2] - 2026-08-05

### Fixed

- **Generated FFI code is clean under edition 2024's stricter lints.** Two more consequences of
  0.54.0's edition bump: the `ffi_set_out_error` helper nested `if let Ok(cs) = …` inside a null
  check, which edition 2024 rejects as `collapsible_if` now that let-chains are stable; and the
  error-method emitter wrote raw-pointer dereferences and `CString::from_raw` as bare statements
  inside `unsafe extern "C"` bodies, which `unsafe_op_in_unsafe_fn` — on by default in 2024 — turns
  into a hard `error[E0133]` for any error type declaring methods. Consumers lint generated crates
  with `-D warnings`, so both broke their builds.
- **The WASM backend no longer emits a duplicate binding for a field and a same-named method.**
  Mirroring the FFI fix in 0.54.1: the field-getter and method-wrapper loops both emitted
  `pub fn <name>` into one `#[wasm_bindgen] impl`, so a type with a `providers` field and a
  `providers()` method failed to compile with `error[E0592]`. The method wrapper is skipped when a
  field getter of that name was already emitted, leaving the getter as the callable surface. A
  survey of the other backends found napi, php, jni, go, dart and java unaffected.

## [0.54.1] - 2026-08-05

### Fixed

- **The generated FFI error accessors compile under edition 2024.** 0.54.0 moved generated Rust
  crates to edition 2024 and converted the FFI templates to `#[unsafe(no_mangle)]`, but the sweep
  missed `error_gen`'s shared emitter, which builds the `status_code`, `is_transient`, `error_type`
  and `error_type_free` functions from Rust string literals rather than templates. Those four kept a
  bare `#[no_mangle]`, which edition 2024 rejects (`unsafe attribute used without unsafe`), so any
  repo with a core error type failed to build its `-ffi` crate after regenerating on 0.54.0.
- **A field and a method sharing a name no longer emit a duplicate FFI symbol.** The field-accessor
  and method-wrapper emitters each minted `{prefix}_{type}_{name}` with no collision check, so a
  type with both a `providers` field and a `providers()` method produced two definitions of the same
  `#[unsafe(no_mangle)]` function (`error[E0428]`). The method wrapper is now skipped when a
  same-named field accessor was already emitted, which keeps the existing symbol and its semantics.

## [0.54.0] - 2026-08-05

### Added

- `crates.readme.languages.<name>.snippet_language` lets a README language borrow its code
  snippets from a differently-named snippet directory (e.g. an `ffi` README pulling examples
  from a `c/` snippet root, since the FFI binding's usage examples are C code and a consumer
  repo already maintains one `c/` snippet set rather than a duplicate `ffi/` one). Defaults to
  the language's own code, so existing configs are unaffected. Only applies to
  `include_snippet(language)` calls using the current README's own language variable — a
  template calling `include_snippet` with an explicit literal (e.g. `include_snippet("python")`)
  is unaffected.

### Changed

- Generated Rust crates (e2e `Cargo.toml`, scaffolded FFI crates) now declare `edition = "2024"`
  instead of `"2021"`, matching every other scaffolded language crate.

### Fixed

- **The generated PHP e2e `composer.json` uses the configured namespace verbatim as its PSR-4
  prefix.** The autoload key was re-derived from the *composer package name* by splitting it on
  `-` and upper-camel-casing each part, so `xberg/html-to-markdown` produced the three-segment
  prefix `Html\To\Markdown\` while the emitted PHP declared the one-segment `namespace
  HtmlToMarkdown;`. The prefix never matched, Composer never autoloaded the facade class, and
  every PHP e2e test failed with `Class "…\HtmlToMarkdown" not found`. A namespace that really
  does contain separators (e.g. `Xberg\Crawlberg`) is still preserved as written.
- **Generated Dart FRB loader code derives its `package:` URIs from `pubspec_name`.** The package
  segment was reconstructed from the bridge crate's file stem (`<crate>_dart` → `<crate>`), so a
  repository whose Dart package is named differently from its Rust crate emitted
  `package:html_to_markdown_rs/src/native_loader.dart` for a package actually named `h2m`. Every
  Dart e2e test failed to load with `Not found: 'package:…/src/native_loader.dart'`. This affected
  the loader import, both `Isolate.resolvePackageUri` calls, and the `dart run …:download_libs`
  hint. The bridge output directory stays crate-derived, since that is a Rust output path.
- The C FFI backend's static-constructor, string-parameter, and trait-bridge registration
  templates now compile under edition 2024. Three emitters (`ffi_opaque_constructor_header.jinja`
  and the `service_api_*`/`registration_variant` templates) still wrote a bare `#[no_mangle]`,
  which edition 2024 rejects outright (`unsafe attribute used without unsafe`). Several
  trait-bridge templates also dereferenced raw pointers (`&*vtable`) and called the `unsafe fn`
  `ffi_set_out_error` without an explicit `unsafe { }` block, which edition 2024 now warns on
  (`unsafe_op_in_unsafe_fn`) even inside an `unsafe fn` body. No generated symbol name, signature,
  or behavior changed.
- Generated shebang scripts keep their executable bit across a regen. `poly fmt` rewrites changed
  files via atomic rename, which resets the mode to `0644`, so every full regen silently stripped the
  bit from the scripts poly reformatted (`run_tests.php`, `download_ffi.sh`, `mvnw`, `gradlew`) and
  poly's own `file-safety` hook then rejected the next commit. The formatting pass now snapshots
  executable modes beforehand and restores any the formatter dropped.
- The generated `credo` pre-commit hook runs `mix deps.get` before `mix credo --strict`. poly runs
  hooks from a staged snapshot outside the repo and Elixir resolves dependencies strictly
  project-locally into a gitignored `deps/`, so credo's own package was missing there and every
  commit touching `.ex`/`.exs` files failed with "Unchecked dependencies for environment dev". The
  snapshot persists between runs, so the fetch is a one-time cost.

## [0.53.1] - 2026-08-04

### Added

- `[workspace.poly] lint-workspace` controls the generated `poly.toml`'s `[lint] workspace` setting.
  Repos whose CI installs only a subset of toolchains need `poly lint` to skip its whole-project
  phase; that setting previously existed only as a hand-edit to the generated file, which the next
  scaffold run silently dropped. Omitting the key emits no `[lint]` table, leaving poly's own
  default in force, so existing output is unchanged.

### Fixed

- A crate-local `Result` alias declared in one module is now honoured by functions in other modules.
  Extraction walks a crate file by file and replaced the alias hint map on every file, so the alias
  from `error.rs` was discarded before the module using it was resolved and its functions fell back
  to `anyhow::Error`. Hints now accumulate across a crate's modules and are reset once per crate, so
  a crate without its own alias no longer inherits the previous crate's error type.
- Swift e2e: a `count_min` assertion on an optional `Vec<Named>` field of a first-class parent DTO
  no longer emits `field()?.count ?? 0`. `emit_vec_struct_serde_getter` collapses that shape to a
  whole-field `-> String`, so the Swift side sees a `RustString` and the generated test failed to
  compile. The countable-vs-JSON-bridged classifier now mirrors the getter emitter's optional split.

## [0.53.0] - 2026-08-04

### Changed

- **An unresolvable README or docs snippet is now a hard error instead of a silent placeholder.**
  `crates.readme.snippets_dir` and `workspace.docs.snippets.dirs` entries that do not exist on disk
  are rejected up front, naming the config key and both the configured and resolved path; a snippet
  reference that cannot be resolved fails the run instead of emitting
  `<!-- snippet not found: ... -->` into the output. The README path previously never failed at all,
  so that placeholder shipped verbatim to package registries while `alef readme` reported success.
  The configured-directory check runs even when no template references a snippet, so a stale path
  cannot hide behind a template that happens not to use the filter.

  **Breaking:** repositories whose snippet references are already broken now fail `alef readme` /
  `alef docs` until the missing snippet files are added or the references removed.

### Fixed

- **Closing code fences in generated API reference docs are no longer tagged with a language.**
  `replace_fence_lang` appended the language to every line starting with a fence, including the
  closing one, turning ` ``` ` into ` ```rust ` and reopening the block instead of closing it. This
  corrupted every `**Example:**` block rendered from a doc comment.
- **A generic `Result<T>` alias now yields the crate's real error type in generated signatures.**
  Hint extraction was gated on the alias having no generic parameters, so the idiomatic
  `pub type Result<T> = std::result::Result<T, MyError>;` was skipped and signatures fell back to
  the placeholder `anyhow::Error`, rendered as a nonexistent `Error` type.
- **Magnus tagged-enum predicate methods emit Ruby booleans.** The value was interpolated through
  minijinja, which stringifies a bool Python-style, producing `def system? = True` — parsed by Ruby
  as a constant lookup, so any predicate call raised `NameError`.
- **Scaffold `.cargo/config.toml` `[env]` structured values render valid TOML booleans.** The
  `relative` flag was interpolated straight from a bool through minijinja, which stringifies it
  Python-style as `True`/`False` — invalid TOML that broke `cargo` on any scaffold using a
  structured env entry (e.g. the Ruby `preferred-ruby.sh` path). The value is now emitted as a
  lowercase `true`/`false` literal.
- **Generated Kotlin Android `build.gradle.kts` no longer stamps a downstream issue reference into
  every consuming project.** The release JNI guard's explanatory comment carried a cross-project
  issue link that no other repository can resolve; the technical rationale is retained.

## [0.52.0] - 2026-08-04

### Added

- **WASM binding crates can declare additional opt-in core features.** The new
  `[crates.wasm].extra_features` list emits each entry as a generated binding-crate feature that
  forwards to the matching core-crate feature without enabling it by default. This supports
  hand-written WASM modules whose `#[cfg(feature = "...")]` gates are not visible in Alef's extracted
  API surface.

### Fixed

- **Swift bindings link the Rust staticlibs by explicit `.a` path.** The generated `Package.swift`
  linked them via a bare `.linkedLibrary(...)`; with both `lib<name>.a` and `lib<name>.dylib` present
  in `target/`, ld64 preferred the `dynamic_lookup` dylib, so swift-bridge glue symbols (e.g.
  `__swift_bridge__$<Type>$_free`) were never linked and the swift test bundle failed to `dlopen`.
  The scaffold now emits a `resolvedStaticLib` helper and links the two Rust staticlibs by absolute
  `.a` path so the linker cannot substitute the sibling dylib.
- **PHP e2e/test_apps autoload path follows the crate move.** The generated composer autoload
  pkg-path defaulted to the historical `../../packages/php`, stale since 0.51 relocated the PHP
  source to `crates/<pkg>-php`; it now derives from the configured php crate output path (falling
  back to `packages/php` when unconfigured).

## [0.51.2] - 2026-08-04

### Fixed

- **Swift e2e: `Option<Vec<Named>>` fields no longer emit non-compiling `.count` assertions.**
  Fields like `elements: Option<Vec<Element>>` are natively bridged by swift-bridge as
  `Optional<RustVec<T>>`, not JSON-bridged to `RustString`. The e2e classifier previously
  treated every optional Vec field as JSON-bridged, so `count_min`/`count_equals`/`min_length`
  assertions emitted `<accessor>().toString().count` against `RustVec<T>?`, which does not
  compile ("value of type 'RustVec<Element>?' has no member 'toString'"). Classification now
  matches the real getter shape used by the Swift binding generator.
- **PHP e2e `composer.json` accepts guzzle 7 or 8.** The generated `require-dev` constraint was
  pinned to `^7.0`, which hard-fails `composer install` against a `composer.lock` that already
  resolved `guzzlehttp/guzzle` to `8.0.0`. The constraint is now `^7.0 || ^8.0`.

## [0.51.1] - 2026-08-04

### Fixed

- Generated Ruby wrappers no longer publish binding types into the global `Object` namespace.
  The previous `Object.const_set` loop exported every module (e.g. `Parser`) globally, colliding
  with unrelated gems such as `parser` (`TypeError: Parser is not a module`). Generated types now
  stay namespaced under their binding module; consumers reference them qualified.

## [0.51.0] - 2026-08-03

### Changed

- PHP userland classes and stubs now honor `[crates.output] php`, co-locating with the generated
  composer.json in the crate (unset config unchanged: `packages/php/`).

## [0.50.0] - 2026-08-03

### Added

- **Configurable logging across alef and its generated bindings.** All of alef's own diagnostics now
  flow through `tracing` (with `error!`/`warn!`/`info!`/`debug!`/`trace!` levels) instead of raw
  `eprintln!`/`println!`, filterable via `-v`/`-vv`/`-q`/`RUST_LOG`. Generated Rust binding glue logs
  host-callback failures through `tracing::warn!` and generated Java bindings through
  `java.lang.System.Logger`, so consuming libraries configure verbosity through their own logging
  setup. Genuine machine-readable command output (JSON reports, schema, diffs, listings) stays on
  stdout through a single sanctioned output helper.
- **A clippy print-guard forbids raw print macros on production code paths.** `print_stdout` and
  `print_stderr` are denied crate-wide (enforced by `poly lint` and the pre-commit hook); the few
  legitimate stdout sites (the output helper, report modules, e2e harness, and test code) carry a
  narrow `#[allow]`.

### Changed

- **Verbosity is reconciled to a single channel.** `-v` now raises the log level to `debug` and `-vv`
  to `trace` (previously `-v` did not change the level); the separate `DispatchContext.verbose` flag
  was removed and its per-file detail folded into `debug!`.
- **Generated Rust crates gain a `tracing` dependency** when trait bridges are present, sourced from
  the centralized version registry (Renovate-managed). The WASM `__log_host_failure` JS-console helper
  was removed in favor of a Rust-side `tracing::warn!`; consumers wanting browser output wire a wasm
  tracing subscriber.

### Fixed

- **Swift `RustBridgeC` target now emits a real object file.** The Swift backend declared
  `RustBridgeC` as a compiled SwiftPM target over a directory that held only `RustBridgeC.h`, with no
  translation unit. `swift build` tolerated the header-only target, but Xcode's XCBuild expected a
  `RustBridgeC.o` and failed to link, breaking every Xcode/iOS consumer of the published SPM package.
  The backend now also emits a minimal `RustBridgeC.c`, so a real object file is produced
  (html-to-markdown#449).

## [0.49.0] - 2026-08-01

### Added

- **Swift binding: `ffi_features` config knob.** The swift-bridge Rust shim's injected FFI-crate
  dependency (`<crate>-ffi`) can now be emitted with `default-features = false` and an explicit
  feature list via the new `[crates.<name>.swift] ffi_features` field. Previously this secondary
  dependency was always emitted in plain `{ version, path }` form, inheriting the FFI crate's default
  features with no way to drop cross-compile-hostile features (e.g. `heic` via `libheif-sys`, whose
  `build.rs` cannot satisfy `pkg-config` under cross-compilation). The primary core dependency's
  `features` / `excluded_default_features` / `target_dep_overrides` do not reach this injection.
  Empty (the default) preserves the previous plain form.

## [0.48.8] - 2026-07-29

### Fixed

- **Swift e2e `.count` assertions no longer emit uncompilable `RustString` accesses.** The Vec-field
  classifier in `build_swift_first_class_map` had dropped the `f.optional` disjunct, so optional
  `Vec<Named>` metadata fields (`headings`/`favicons`/`hreflangs`) — which the swift-bridge layer
  JSON-bridges to a `-> RustString` getter with no `.count` — were recorded as countable and emitted
  `headings()?.count`, failing to compile. Restore the disjunct: optional vecs are skipped while
  non-optional vecs (`urls`, `nodes`, `tables`) stay countable.

## [0.48.5] - 2026-07-27

### Added

- **Generated Zig e2e projects now expose a dedicated `smoke` build step** (`zig build smoke`) that
  runs `smoke_test.zig` in isolation, outside the serial test chain, as a fast published-package
  sanity check. Zig 0.16's `zig build` has no `--test-filter`, so the isolation is wired as its own
  build step with its own `RunStep` over the same compiled binary; it is emitted only when a
  `smoke_test.zig` fixture exists, so no dead step is generated.

### Fixed

- **The Dart flutter_rust_bridge loader is now upgraded in place when a stale one was injected by an
  older alef.** The marker-based idempotency check previously froze any already-injected loader
  forever, so a binding shipped with a cache-unaware loader never picked up the fix on regeneration:
  the download script populated the versioned cache, but the frozen loader never looked there. A file
  carrying the loader marker but not the current-template sentinel (`nativeCachedLibPath()`) now has
  its injected region replaced with the current template while preserving the original `init` body.
- **Zig e2e dependency resolution now treats repeated-character fill hashes (`AAAA…`) as placeholders,
  not just the explicit `STALE_HASH_REGENERATE` marker.** Such fills — used to keep `build.zig.zon`
  syntactically valid before a release exists to hash — were being emitted as real dependency hashes,
  failing `zig build` with a hash mismatch. They now fall through to the cache/network/omit-hash path.
  The heuristic requires a run of at least 16 identical characters, so it cannot misfire on a genuine
  base64 content multihash.

## [0.48.4] - 2026-07-27

### Fixed

- **C# NuGet packing no longer fails on a missing `runtime.json`.** `scaffold_csharp` now emits
  `packages/csharp/<Namespace>/runtime.json.template` alongside the csproj — the file the csproj's
  `RequireRuntimeJson` target has always required but that nothing ever generated, so every consumer
  `dotnet pack` errored. The template carries NuGet's RID-fallback graph (one `<PackageId>.runtime.<rid>`
  dependency per enabled published RID, plus `linux-musl-*` `#import` fallbacks) with a literal
  `{{VERSION}}` placeholder that CI substitutes before pack.
- **The generated Maven pom's enforcer floor no longer exceeds the CI runner's Maven version.**
  `MAVEN_CORE` (which feeds `<requireMavenVersion>`) had been renovate-bumped to `3.9.16`, above the
  `3.9.11` GitHub-hosted runners ship, so `enforce-maven` failed during publish. It is now a fixed
  compatibility floor (`3.6.3`) with the `renovate:` annotation removed so it is not auto-bumped again.

## [0.48.3] - 2026-07-26

### Fixed

- **Magnus RBS stubs now emit the real owning class for `Self`-returning methods instead of the
  `json_value` fallback.** When a type is managed by another codegen pass (e.g. a service owner
  type that is `binding_excluded`) it is still emitted as a `class` stub here, so builder-style
  methods and constructors returning `Self` (resolved to the owning type during extraction) must
  reference that class. A new `substitute_excluded_types_except_owner` never substitutes the owner
  type, restoring `-> App`-style return types (regression from 0.42→0.48).
- **Generated Go service templates no longer leave errors assigned to the blank identifier**, so
  they pass `golangci-lint` with `errcheck.check-blank = true`. The background `Run()` goroutine and
  the TCP readiness probe's `conn.Close()` in `service_start_background.jinja`, and the error-branch
  `json.Marshal` in `service_handler_registry.jinja`, now check their errors explicitly.

## [0.48.2] - 2026-07-26

### Fixed

- **A full regen (`alef all`) now converges to a zero-drift tree** instead of needing 2-3 manual
  `poly fmt --fix` passes downstream. `poly fmt --fix <root>` now loops to a fixed point (bounded
  at 3 passes, detected via `poly fmt --check`) — some poly-bundled engines (`.cs`, `.java`,
  `.json`) were not single-pass idempotent on freshly generated output.
- **Rust crates are no longer left rustfmt-dirty after a full regen.** A workspace-wide `cargo fmt
  --all` now runs (best-effort, skipped with a warning when `cargo`/`rustfmt` are unavailable),
  folded into the same convergence loop as `poly fmt` so any drift it introduces is reconciled by
  the next pass.
- **Cargo-sort now covers every crate in the workspace on a full regen, not just the languages
  that happened to be generated.** The old per-language cargo-sort residuals only ran for
  wasm/ffi/ruby/elixir/R, and the workspace-wide (`-w`) variant only ran when the ffi target was
  present — leaving python, node, php, swift, and dart binding crates unsorted and tripping poly's
  own bundled cargo-sort check. A full regen now runs a single `cargo sort -n -w` at the repo root
  covering the whole workspace regardless of target languages (partial/single-language regens keep
  the existing per-language residuals unchanged).

### Changed

- `format_generated`'s full-regen path (`only_languages = None`, used by `alef all`) now converges
  `poly fmt`, `cargo fmt`, and workspace-wide `cargo sort` together in one bounded loop instead of a
  single `poly fmt` pass plus fixed per-language residuals.

## [0.48.1] - 2026-07-26

### Fixed

- **Generated C# `.csproj` no longer embeds a downstream project name.** The thin meta-package
  `.csproj` template carried a comment referencing a specific consumer project's issue tracker
  (`xberg #1280`), leaking a downstream project name into every generated csproj and tripping alef's
  project-agnosticism enforcement. The internal issue references are removed from both the source
  doc comment and the emitted csproj comment.

## [0.48.0] - 2026-07-26

### Added

- **cbindgen C headers are formatted by poly.** When an FFI target is present, the generated
  `poly.toml` enables poly's `clang-format` catalog tool (`[tools.clang-format] enabled = true`) and a
  canonical `.clang-format` is scaffolded, so `poly fmt` and the pre-commit hook format the
  build-time-generated `crates/*-ffi/include/*.h` headers consistently across repos.
- **Per-language lint defaults extended so consumer repos can drop identical `[crates.lint.*]`
  overrides:** ruby runs `bundle install` before rubocop, and elixir runs `mix deps.get` before credo.

### Changed (BREAKING)

- **Removed the hidden `--format` flag** from `alef generate` / `all` / `init` / `e2e generate` /
  `test-apps generate`. Formatting always runs, delegating to `poly fmt` whenever poly is on PATH; when
  poly is absent, generation now warns and continues (emitting unformatted output) instead of aborting.
- **ktfmt (`--kotlinlang-style`) is now the single Kotlin formatter** for both the `kotlin` and
  `kotlin_android` backends (was `gradle ktlintCheck`), and the **Swift default formats only `Sources`**
  (not `Tests`). Both match what every consumer repo already overrode to; regenerating changes the
  Kotlin, Kotlin-Android, and Swift lint commands.

### Fixed

- **Swift: generated `Package.swift` links `libbz2`** at both the dev and artifactbundle sites, fixing
  undefined `_BZ2_bzDecompress*` symbols in the RustBridge target.
- **Python `.pyi` enum stubs no longer emit `# noqa: PYI029`** on the generated `__str__`/`__repr__`
  stubs. PYI029 is not enabled in the generated ruff config, so ruff flagged the suppression itself as
  an unused directive (`RUF100`).
- **Kotlin-Android `build.gradle.kts` formatting cleaned up.** The host-JNI `else` branch keeps a
  single-spaced trailing `// linux` comment (was double-spaced), and the
  `mavenPublishing { configure(...) }` call wraps its multi-line `AndroidSingleVariantLibrary(...)`
  argument onto its own line.
- **Ruby `.rbs` stubs no longer reference undeclared types (`steep RBS::UnknownTypeName`).** Streaming
  methods now declare `Enumerator[<ItemType>]` from the adapter's real item type instead of an
  undeclared `<Method>Iterator`, and any signature referencing a binding-excluded or opaque
  (`alef(skip)`) type substitutes the declared `json_value` alias.
- **Ruby `.rbs` trait-typed parameters/returns now reference the interface name.** A parameter or
  return whose type is a trait was emitted with the bare trait name (e.g. `DocumentExtractor`), but
  traits are surfaced only as host-implementable `interface _TraitName` declarations, so `steep`
  failed with `RBS::UnknownTypeName`. Such references are now substituted to their `_`-prefixed
  interface name.
- **Python PyO3 trait-bridge Protocol methods with numeric returns are typed `Iterable` (#203).**
  A Protocol method is implemented by the host and its return extracted by the bridge, so typing it
  with the parameter rule (e.g. `Vec<Vec<f32>>` → `list[list[float]]`) rejected NumPy values the
  bridge already accepts, forcing a `.tolist()` at every call. Numeric `Vec` returns now render as
  `Iterable`; only numeric leaves widen, and parameters and ordinary function stubs are unchanged.

### Removed

- **PMD/CPD dropped from the generated Java package.** PMD ran the built-in `quickstart` ruleset
  (the emitted `pmd-ruleset.xml` was never referenced by the `pom.xml`), and PMD/CPD mostly fought
  alef-generated code. The `pmd` workspace hook, the `maven-pmd-plugin` build plugin and its
  `pmd.skip`/`cpd.skip` publish-profile properties, and the scaffolded `pmd-ruleset.xml` are all
  removed. `checkstyle` continues to run as before.
- **ktlint removed entirely from generated Kotlin and Kotlin-Android projects.** ktfmt is the single
  Kotlin formatter, and ktlint's rule set fought ktfmt's output on generated code. The
  `org.jlleitschuh.gradle.ktlint` gradle plugin (and its `ktlint {}` config block), the `ktlint`
  `poly.toml` workspace hook (`gradle ktlintCheck`), and the `ktlint_standard_*` `.editorconfig`
  overrides are all removed from both backends.

## [0.47.2] - 2026-07-25

### Fixed

- **Generated Go binding is cgo-safe again.** The `// If linking fails … cannot find -lxberg_ffi …` note
  was emitted directly above the `/* #cgo … */` preamble, so cgo fed it to the C compiler
  (`error: unknown type name 'If'`, stray backtick) and every cgo build failed. The note is now
  separated from the cgo preamble by a blank line.

## [0.47.1] - 2026-07-25

### Fixed

- **C# meta-package is thin again (fixes NuGet HTTP 413 on publish).** The generated
  `packages/csharp/<Namespace>/<Namespace>.csproj` packed the entire native closure via
  `<None Include="runtimes/**">`, pushing the `XbergIo.Xberg` meta package past NuGet's size limit
  (HTTP 413; regressed since rc.37 — the per-RID split from #1280/rc.35 had slimmed it). The template
  now packs only `runtime.json` — the RID-fallback graph rendered from `runtime.json.template` by CI —
  plus the managed assembly, and adds a `RequireRuntimeJson` pre-pack target that hard-errors if
  `runtime.json` is missing. Native closures continue to ship in the per-RID
  `<PackageId>.runtime.<rid>` packages.

## [0.47.0] - 2026-07-25

### Fixed

- **Python `__all__` now honors `exclude_functions`.** An excluded function leaked into the generated
  `__init__.py` `__all__` even though it was correctly kept out of the `.api` import list — most
  visibly an excluded `*_async` variant whose sync sibling was already dropped. The undefined name in
  `__all__` tripped pyrefly's `bad-dunder-all` (now enforced by `poly lint .`) and would break
  `from <pkg> import *`. The `__all__` builder applies the same exclude filter as the import list.

## [0.46.0] - 2026-07-25

### Added

- **poly is now the single lint orchestrator: `poly lint .` invokes the external linters poly does not
  bundle.** The generated `poly.toml` emits a `workspace = true` hook per configured language for the
  tools poly has no built-in engine for — pyrefly (Python type-check), rubocop + steep (Ruby),
  golangci-lint (Go), checkstyle + pmd (Java), ktlint (Kotlin/Android), `dart analyze` (Dart), and
  credo (Elixir). Each runs once over its package directory, discovers its own native config
  (`.rubocop.yml`, `.golangci.yml`, `checkstyle.xml`, `.credo.exs`, `analysis_options.yaml`, …), and is
  skipped gracefully when its toolchain is absent. The existing `pyrefly` hook gains `workspace = true`
  so it actually runs during `poly lint .` (previously it only fired on git pre-commit). Downstream
  repos can drop their per-language lint tasks in favour of `poly lint .`.

### Changed

- **Python generated `pyproject.toml` no longer declares a `ruff` dev-dependency.** poly bundles ruff
  for lint+format, so a standalone `ruff` in the dev group is redundant; only the `pyrefly`
  type-checker (which poly does not provide) remains.
- **Python `poly.toml`: `[lint.python.ruff]` now uses an explicit `select` allowlist** instead of
  `select = ["ALL"]` minus an ignore list. Enabling every rule then suppressing the noise meant each
  ruff release could silently start firing a new deny-by-default rule (e.g. the `CPY` copyright-header
  family) on generated bindings. The scaffold now selects the rule families we want; families that were
  only ever carried to be fully ignored (`COM`, `FBT`, `FIX`, `TD`, `PD`, `EM`, `TRY`, `BLE`) are no
  longer selected, and `ignore` is trimmed to the in-family sub-rules that remain relevant.

## [0.45.0] - 2026-07-25

### Added

- **Go backend: download-at-consume native distribution.** Published Go modules no longer require
  native libraries inside the module (module zips only contain the git tag's files; `.lib/` stays
  gitignored). The generated `cmd/setup` tool replaces `cmd/download_ffi`: it downloads the platform
  FFI library from the GitHub release into a versioned user cache
  (`os.UserCacheDir()/<name>/go/<version>/<platform>`), verifies its SHA-256 sidecar, and writes a
  machine-local, gitignored cgo link shim (`<name>_cgo_link.go`) with absolute `-L`/`-rpath` flags
  into the consumer's package. The binding exports a per-version `RequireNativeSetup_<version>`
  sentinel referenced by the shim, turning shim/module version skew into a compile error.
  `embed_ffi.go` now embeds only `include/*` so `go mod vendor` carries the C header; `go generate`
  runs `cmd/setup -lib-dir .lib`; test-app run defaults use `go run <module>/cmd/setup` instead of
  the copy-module-out-of-cache workaround.

### Fixed

- **PHP: registry-mode e2e `composer.json` now declares the userland PSR-4 autoload.** Only the
  `Local` dependency mode emitted the `"autoload"` section mapping the binding's PHP namespace to
  the local `packages/php/src/`, so registry-mode test apps could not resolve the userland classes
  layered over the native ext-php-rs extension — every test failed with `Class not found` even after
  PIE installed the extension. Both modes now emit the mapping via a shared helper.

## [0.44.0] - 2026-07-24

### Fixed

- **Swift: link the C++ standard library in the generated `Package.swift`**: the Rust staticlib pulls in
  C++ dependencies (onnxruntime, tesseract, ClipperLib) whose C++ ABI symbols (`__cxa_throw`,
  `__gxx_personality_v0`, `__cxa_guard_acquire`, …) were left undefined at the SwiftPM link step, so
  consuming a published Swift package failed to link. Both the in-tree and the published
  `.binaryTarget` root manifests now link `c++` on Apple platforms and `stdc++` on Linux,
  platform-conditionally.

- **Renovate now actually maintains the generated dependency version pins**: the `renovate.json`
  regex customManager targeted a stale path (`crates/alef-core/src/template_versions.rs`, gone
  since the crate went root-flat in 0.18.0) and required `// renovate:` marker comments that no
  const carried, so it bumped nothing. The path is corrected to `src/core/template_versions.rs`
  and every auto-bumpable const now carries a `datasource`/`depName` marker. An explicit top-level
  `"enabled": true` re-enables the repo (a closed onboarding PR had left it flagged disabled). Pins
  no longer drift stale, which is what was driving Dependabot churn (jackson, guzzle, junit, …) in
  the generated `/packages/*` and `/e2e/*` directories of consumer repos.

- **Generated dependency versions are fully centralized in `template_versions.rs`**: several
  versions were hardcoded outside the registry and had drifted. The Java scaffold `pom.xml` (a raw
  `format!` string, also a jinja-templates rule violation) is converted to a Minijinja template and
  sources every version from `template_versions::maven`/`toolchain` (fixing stale jackson `2.21.2`,
  junit `5.11.4`, and six maven-plugin pins). The Java e2e pom template (`org.jetbrains:annotations`,
  `maven-antrun-plugin`), Python e2e (`pytest`/`pytest-asyncio`/`pytest-timeout`/`setuptools`), Gleam
  e2e (`gleam_http` range), Dart scaffold (`http`, `crypto`), and Rust e2e (`serde`/`serde_json`/
  `tokio`) now all draw from the central consts.

- **Renovate marker datasources corrected so no pins error out**: the Dart pins used
  `datasource=pub`, but Renovate's Dart datasource id is `dart` — the invalid id produced
  "Missing datasource" / "Unsupported range strategy" warnings and blocked those bumps. The Gradle
  plugin pins (`ktlint-gradle`, `gradle-versions-plugin`, `gradle-maven-publish-plugin`) resolve
  from the Gradle Plugin Portal rather than Maven Central, so a `registryUrls` package rule points
  their maven lookups there (fixing the `ktlint-gradle: no-result` lookup failure). Renovate has no
  CRAN datasource, so the `rextendr` pin is now manually tracked (marker removed) rather than
  emitting a "Missing datasource" warning, and the custom manager carries an explicit
  `rangeStrategy: replace`. The Ruby gem pins used pessimistic (`~>`) constraints that
  Renovate's regex custom manager cannot bump — the `ruby` versioning then logged an
  "Unsupported range strategy" warning and produced no update — so those markers are removed
  and the gems are tracked manually (their `~>` floors already admit newer releases at
  `bundle install`).

## [0.42.1] - 2026-07-22

### Added

- **Node (NAPI): the ergonomic `/service` module re-exports the native value types it wraps**: the
  generated `service.ts` exported only the service class, so consumers (and e2e harnesses) importing
  from `<pkg>/service` could not reach the `Method` enum or `RouteBuilder` the service API expects —
  a `Method`-is-`undefined` `TypeError` at runtime. The service module now also re-exports the native
  value types referenced by the service surface, skipping its internal aliased self-import.

- **Ruby (Magnus): ABI-aware native extension loading and staging**: the generated `native.rb`
  wrapper now resolves the compiled extension through `RbConfig` — searching ABI-specific candidates
  (`lib/<ext>/<ruby_version>/...`) across `DLEXT`/`DLEXT2`, with the legacy flat path as a fallback —
  and raises a `LoadError` listing every expanded candidate when none match. The Ruby packager stages
  native libraries under `lib/<ext_name>/<ruby_abi>/...`, deriving the ABI from `RbConfig`'s
  `ruby_version` unless `RUBY_ABI` is set. This makes alef the canonical place for multi-ABI Ruby
  distribution. A `RUBY_ABI` override is now trimmed and rejected when blank, and a failing `ruby`
  invocation surfaces its stderr for diagnosability.

### Fixed

- **Elixir e2e: ExUnit test names are bounded to stay under the 255-character limit**: a fixture with
  a long description produced a computed test name (`test {describe} {description}`) of 255+ characters,
  which ExUnit rejects with `SystemLimitError`, failing the whole suite at compile time. The description
  portion of the test name is now truncated on a UTF-8 char boundary to keep the full name under the
  limit; each describe wraps a single test, so names remain unique.

- **`[crates.exclude].fields` now applies to external type roots**: fields hidden globally were only
  pruned from the primary crate's surface, so a field on an externally-extracted DTO root could pull
  in a colliding foreign type and fail merge with a same-name host conflict. Excluded fields are now
  applied to each external type root before its DTO roots are expanded, matching the behavior on the
  primary surface.

## [0.39.0] - 2026-07-20

### Added

- **WASM: configurable `wasm-opt` pass via `[crates.wasm].wasm_opt`**: the generated wasm binding
  `Cargo.toml` hard-coded `[package.metadata.wasm-pack.profile.release] wasm-opt = false`, so
  wasm-pack always skipped the size-optimization pass. A new `wasm_opt` field (a list of `wasm-opt`
  flags, e.g. `["-Oz"]`) is now emitted as `wasm-opt = [...]` when set, letting large wasm builds
  stay under CDN per-file size caps. Defaults to empty, which still emits `wasm-opt = false` — the
  historical behavior is unchanged for consumers that don't set it.

## [0.38.4] - 2026-07-20

### Fixed

- **Ruby (Magnus): `&mut self` methods on opaque types are now bound**: the module-init
  registration loop unconditionally skipped every `RefMut`-receiver method, so an opaque type whose
  methods all take `&mut self` (e.g. a tree-sitter `Parser` with `set_language`/`parse`/`parse_bytes`/
  `reset`) was exposed to Ruby with zero callable methods. Opaque wrappers are `Arc<Mutex<T>>` and
  their instance methods already delegate through the lock, so these methods now register. The gate
  stays scoped to opaque types — non-opaque by-value DTOs have no delegating wrapper and are
  unchanged.
- **Ruby (Magnus): a `Bytes` parameter now decodes from a Ruby `String`**: the generated `.rbs`
  advertised `String` but the wrapper took `Vec<u8>` (a Ruby `Array`), so a `bytes` argument such as
  `parse_bytes(source)` could not be called with a String. Magnus `Bytes` params now take
  `magnus::RString` and copy into a `Vec<u8>` before the core call, matching the advertised contract.
- **Swift: `count_min`/`count_equals` assertions on an opaque-parent `Optional<RustVec<T>>` field**
  now count the decoded array directly instead of `.toString().count`, which counted characters of a
  JSON string rather than elements.
- **Swift: DTO `CodingKeys` now honor serde `rename`/`rename_all`**: a discriminant field renamed at
  the serde layer (e.g. `call_type` serialized as the wire key `type` on a tool-call variant) is now
  decoded from its wire key instead of throwing `keyNotFound` on a key the payload never contains.
- **Swift: `Optional<Vec<struct/tagged-enum>>` accessors no longer double-encode**: the
  `getter_vec_enum_string_optional` template encoded each element to a JSON string and collected an
  array of JSON strings, which Swift's strongly-typed `[T]?` `init(from:)` rejected with a
  `typeMismatch` (expected object, found string). The accessor now serializes the field directly via
  `serde_json`. The non-optional `Vec<String>` path (which strips quotes element-wise) is unchanged.
- **Elixir: the e2e generator no longer appends an `_async` suffix to streaming entry points**
  (e.g. `chat_stream`), which produced calls to a nonexistent `chat_stream_async/2`
  (`UndefinedFunctionError`). The binding was always correct; only the generated e2e was wrong.
- **Internal: removed downstream project-name references from enforced source files** (a Swift
  forwarder test's error-type fixture and a conversions doc-comment example) so the
  project-agnostic guard passes.

## [0.38.3] - 2026-07-20

### Fixed

- **Go: free-function name no longer collides with a same-named type**: when a Rust crate exposed
  both a free function and a struct that mapped to the same Go PascalCase identifier (e.g.
  `model_info` / `ModelInfo`), the Go backend emitted both `func ModelInfo(...)` and
  `type ModelInfo struct`, which the Go compiler rejects as a redeclaration. Free functions whose
  Go name collides with a generated type name are now `Get`-prefixed (`GetModelInfo`); the type name
  and the underlying C FFI symbol are unchanged.
- **Ruby (Magnus): enum data-variant `Map` fields flattened to `String` now round-trip via JSON**:
  Magnus collapses a `Map` field on an enum data-variant to a JSON `String` DTO field, but the
  generated `From` impls still emitted the `HashMap::into_iter().map(...).collect()` template,
  producing uncompilable Rust (`into_iter` on `String`). Such fields now round-trip via
  `serde_json`. Struct `Map` fields (which Magnus keeps as native `HashMap`) are unaffected.
- **Swift: free functions returning a `String`-backed enum no longer emit an invalid initializer**:
  the forwarder used the struct positional-init template (`EnumType(_rb_obj)`) for enum returns,
  but a `String`-backed enum only synthesizes `init(from:)`. Enum returns now decode via the enum's
  `RawValue` initializer, matching the existing enum-typed DTO-field pattern.

### Added

- **Multipart request-body synthesis for TestClient-driven languages**: the shared `http_call`
  driver (Go, Zig, Gleam) now synthesizes a `multipart/form-data` request body from the handler's
  object body schema when a fixture declares that content type but carries no explicit body —
  matching the Python/Ruby/TypeScript generators. Previously these languages emitted an empty
  request body, so the core rejected multipart upload fixtures with 422 before the handler ran.

## [0.38.1] - 2026-07-19

### Fixed

- **`alef all --clean` now poly-formats root-level generated files**: the full-regen format pass
  only ran `poly fmt --fix` over each language's package directory, so generated files that live
  outside every package dir — `poly.toml`, `.cargo/config.toml`, and the docs/skills output — were
  never formatted and failed `poly fmt --check` in consuming repos (0.38.0 regression). A `--clean`
  run now formats the whole base directory.

## [0.38.0] - 2026-07-19

### Added

- **`[crates.ruby] required-ruby-version` config**: the scaffolded gemspec's
  `required_ruby_version` constraint is now configurable per repo. Unset, it defaults to
  `">= 3.2.0"` (see Fixed).
- **`[[workspace.poly.hooks-sources]]` passthrough**: external git-sourced pre-commit hook
  sources (e.g. an `ai-rulez` validation hook pinned by `git` + `revision`) are now modeled in
  `[workspace.poly]` and rendered as `[[hooks.sources]]` blocks in the generated `poly.toml`, so
  consumers relying on such hooks no longer have to hand-edit the generated file (which regen
  would clobber). Empty by default — output stays byte-identical when unused.

### Fixed

- **Ruby gemspec no longer pins `< 4.0`**: the scaffolded gemspec hardcoded
  `required_ruby_version = [">= 3.2.0", "< 4.0"]`, blocking `gem install` on Ruby 4.x. It now
  defaults to `">= 3.2.0"` (no upper bound). Affects every repo with a Ruby binding.
- **Elixir positional JSON-encoded NIF args now handle `nil` and pre-encoded strings**: the
  positional constructor arg (e.g. `create_engine/1`) unconditionally re-encoded via
  `Jason.encode!`, so a `nil` default config encoded to `"null"` and a pre-encoded JSON string
  (the documented `Jason.encode!(%Struct{})` form) double-encoded — both rejected by serde at the
  NIF boundary. The generated wrapper now forwards `nil` and binaries as-is and encodes native
  terms, mirroring the keyword-arg path. Affects all rustler bindings.
- **Generated Rust e2e test code is now clippy-clean under `--all-targets`**: the `min_length`
  assertion emitted `x.len() >= 1` (trips `clippy::len_zero`); for `n == 1` it now emits
  `!x.is_empty()` and keeps `len() >= n` for `n > 1`. The generated mock-server `Child` singleton
  is annotated `#[allow(clippy::zombie_processes)]`.

### Changed

- **Dependencies bumped to latest**: `syn` `2` → `3` and `jsonschema` `0.46` → `0.48`. The `syn` 3
  upgrade restructured `ItemImpl.trait_` (3-tuple → `(Path, For)`) and `Receiver` (`reference`/
  `mutability` → `kind: ReceiverKind`); the Rust-source extractor was adapted accordingly. No
  change to generated output.

## [0.37.2] - 2026-07-19

### Fixed

- **Swift e2e `.length`/`.count` assertions on JSON-bridged collections no longer emit
  uncompilable `.count`**: a length/count/size assertion whose collection leaf is a swift-bridge
  scalar `RustString` getter — an `Option<Vec<T>>`, `Map`, or `Vec<Vec<_>>` field, which bridges
  to a single JSON string with no `.count` — generated `<collection>()?.count`, which does not
  compile. Such assertions are now skipped with a "not available on result type" comment,
  matching the go/csharp/java backends. Countable `RustVec` getters (plain `Vec<T>`) are
  unaffected and still emit `.count`.

## [0.37.1] - 2026-07-19

### Fixed

- **Elixir streaming NIFs now compile**: the generated Rustler streaming start NIF
  (`crawl_stream`/`batch_crawl_stream`-style methods on an opaque resource) cloned the
  `Arc<RwLock<Handle>>` and called the core stream method on it, which does not exist
  (`E0599`). Streaming codegen now read-locks and clones the inner handle first, matching the
  non-streaming opaque method path.
- **Swift `Option<Vec<serde-struct>>` getters on opaque parents no longer collapse to `String`**:
  an optional `Vec` of a serde-deriving struct on an opaque (non-first-class) parent was
  JSON-degraded to a single `RustString` getter while the constructor kept a real
  `Optional<RustVec<T>>`, so `.field()?.count` did not compile. The getter now returns
  `Option<Vec<T>>` (matching the constructor and the opaque element accessors); the JSON
  degradation is retained only for first-class Codable parents whose Swift decoder needs it.
- **Project-agnostic fixtures**: renamed real downstream project names used as sample fixtures in
  `src/core/ir/surface.rs` and the C# e2e test-app generator to neutral names, restoring the
  project-mention guard to green.

## [0.37.0] - 2026-07-19

### Added

- **`custom_modules` entries for backends that ignore them are now flagged** (#183): `alef generate`
  emits a warning when `[custom_modules].<lang>` carries entries for a language whose backend never
  consumes them (`node`, `wasm`, `go`, `java`, `csharp`). Only pyo3, ffi, php, magnus, rustler, and
  extendr read `custom_modules`; entries elsewhere silently did nothing. The warning names the
  language and, for wasm, points at `[crates.wasm].custom_rust_modules` — the knob that actually
  declares hand-written Rust modules. The misleading `custom_rust_modules` doc comment (which
  claimed `[custom_modules].wasm` adds TypeScript re-exports) is corrected.
- **`alef verify` flags hash-inconsistent trees** (#184): verify now reports when the generated tree
  carries more distinct `alef:hash` values than there are generating crates — the signature of a
  partial regeneration where some files were regenerated and others left with an older hash. The
  check is host-independent (it never recomputes the inputs hash), so partial regens are caught at
  commit time regardless of environment. Surfaces under `--exit-code`.

### Changed (BREAKING)

- **Generation fails fast when a required formatter is missing** (#184): `alef generate` and
  `alef all` now abort up front if `rustfmt`, `poly`, or (for languages with a cargo-sort residual:
  wasm/ffi/ruby/elixir/r) `cargo-sort` is not on `PATH`, instead of warning and emitting
  differently-formatted, host-dependent output. The error names each missing tool and how to install
  it. This makes generation deterministic modulo the config; install the listed tools to proceed.
- **Generated node/e2e dependency bumps**: `@napi-rs/cli` `^3.6.2` → `^3.7.3` (devDependency and the
  default build command), `@types/node` `^22.10.2` → `^26.0.0`, and `vitest` `^4.1.5` → `^4.1.10`.

### Fixed

- **`alef generate --lang <one>` no longer deletes other languages' output** (#178): the orphan
  sweep computed its keep set from the filtered language but widened its roots unconditionally
  (always including `packages/wasm` and `packages/typescript`), so a filtered run deleted every other
  binding's still-valid generated files. Filtered runs now scope the sweep roots to the requested
  languages' own directories; unfiltered `alef all` behavior is unchanged.
- **`alef all` no longer deletes the generated docs reference tree based on host state** (#184): the
  set of reference pages `generate_docs_stage` emits varies with the host (CLI/MCP source presence,
  doc-language subset), so a host that regenerated fewer pages let orphan cleanup delete the
  committed pages it did not produce. Committed pages under `[docs].reference_output` are now
  protected from orphan cleanup.

## [0.36.2] - 2026-07-13

### Fixed

- **Generated test apps had four runtime-breaking defects when run against published packages**:
  - **C# registry test app referenced the wrong NuGet id**: `render_csproj` emitted
    `<PackageReference Include="{project_name}">` (the C# assembly/namespace, e.g. `Xberg`) instead
    of the published NuGet id from `[crates.csharp].package_id` (e.g. `XbergIo.Xberg`), so
    `dotnet restore` failed with `NU1101: Unable to find package`. The registry-mode reference now
    resolves `package_id` → namespace → project name.
  - **Go test app's `go.mod` was an incomplete dependency graph**: only `github.com/stretchr/testify`
    was required, with none of its transitive deps, so `go test` aborted demanding `go mod tidy`.
    `render_go_mod` now emits testify's pinned indirect deps (`go-spew`, `go-difflib`, `yaml.v3`) as
    an `// indirect` block so the app builds offline without a manual tidy.
  - **Dart test app never fetched its native library**: the `download_libs` invocation had been
    dropped on the false premise that natives ship via pub.dev (they exceed pub.dev's 100 MB cap and
    are fetched from the GitHub release). Restored: the run config derives the under-test package name
    and runs `dart run <pkg>:download_libs` between `pub get` and `dart test`, so `RustLib.init()`
    finds the native.
  - **WASM/node test apps shipped a stale JS lockfile across `--clean`**: `pnpm-lock.yaml` pinned an
    older version than `package.json` wanted, tripping pnpm's `minimumReleaseAge` supply-chain gate.
    JS lockfiles (`pnpm-lock.yaml`, `package-lock.json`, `yarn.lock`) are no longer preserved across
    `--clean` for `node`/`wasm`, so the post-generate `pnpm install --lockfile-only` regenerates them
    fresh; non-JS locks are still preserved.

## [0.36.1] - 2026-07-13

### Fixed

- **`alef docs` over-documented `#[cfg(feature = "…")]`-gated items for feature-restricted bindings**:
  the reference-docs generator rendered the full extracted API surface without evaluating each
  binding's effective feature set, so a binding whose feature set excludes a gate (e.g. the wasm
  binding — `wasm-target`, which does not enable `tree-sitter`) still documented the gated types,
  struct fields, enum variants, and functions, diverging from the surface the binding actually
  compiles. `generate_lang_doc` now filters the surface through the new
  `ApiSurface::with_cfg_filtered_deep` — which drops cfg-gated *members* (fields, enum variants,
  variant fields), not just top-level items — using each backend's real effective feature set
  (Swift/Dart force-enable every cfg-referenced feature minus `excluded_default_features`; other
  backends use their configured feature list). `cfg_feature_satisfied` gains three-valued (Kleene)
  evaluation with full `all`/`any`/`not` and nested-predicate support, and keeps any item whose gate
  depends on an unresolved non-feature leaf (e.g. `target_arch`), so target-conditional items are
  never wrongly dropped.

## [0.34.7] - 2026-07-10

### Fixed

- **dart native loader emitted unparseable Dart (`\${...}` instead of `${...}`)**: the
  `StateError` raised on a full native-library cache miss escaped `${nativeCacheDir() ...}` and
  `${nativeAssetUrlBase()}` as a literal `\$` instead of real Dart string interpolation. The stray
  backslash meant the enclosing single-quoted string terminated early at the nested
  `'<unresolved cache dir>'` literal, producing bare identifiers (`unresolved`, `cache`, `dir`)
  that fail to compile in every consumer of `frb_generated.dart`. Fixed in
  `frb_init_prologue_replacement`; added a regression test asserting real interpolation.
- **e2e shebang scripts lost their executable bit after formatting**: the scaffold writer chmods
  generated shebang scripts (e.g. `run_tests.php`) to `0o755`, but the subsequent `poly fmt --fix`
  pass in the e2e formatter rewrites them via atomic rename, resetting the mode to `0o644`. The
  generated suites then committed a non-executable `run_tests.php`, which trips the
  `check-shebang-scripts-are-executable` file-safety hook downstream. `run_formatters` now
  re-asserts the shebang chmod after every formatter pass, so shebang e2e scripts stay executable.

## [0.34.5] - 2026-07-09

### Added

- **dart native loader**: the Dart backend now generates a runtime loader that fetches the
  platform-matched native from the package's GitHub Release (version-pinned, SHA-256 verified)
  into a versioned user-cache dir on first use, instead of bundling all-platform natives in the
  published package. Adds a shared `native_loader.dart` helper, a cache-resolution loader stage
  that errors actionably on a full miss (naming the asset URL and the `download_libs` / env-var
  escape hatches), and the `crypto` dependency for SHA-256 verification.

### Fixed

- **cargo-machete false positives on binding scaffolds**: the R (extendr), Dart, and Ruby crate
  manifests declare `async-trait` — and Ruby additionally declares `tokio` — for trait-bridge
  support, but a synchronous trait bridge (e.g. a visitor) never imports them in the generated
  shim, so `cargo-machete` flagged them as unused and failed `poly lint`. Each generator now adds
  the emitted-but-unused dependency to its `[package.metadata.cargo-machete]` ignored list: R gains
  the stanza (it previously emitted none), Dart appends `async-trait` (its bridge genuinely uses
  `tokio`), and Ruby appends `async-trait` plus `tokio` when the bridge carries no real async. This
  removes the need to hand-patch the generated manifests after regeneration.

## [0.34.4] - 2026-07-09

### Fixed

- **java visitor codegen**: the upcall `FunctionDescriptor` for visitor callbacks now declares
  `ValueLayout.JAVA_INT` as its return layout, matching the `int`-returning `handleVisit*` bridge
  methods and the `int.class` `MethodType`. It previously emitted `ValueLayout.JAVA_LONG`, so the
  Java Linker rejected every visitor upcall stub with `IllegalArgumentException: Wrong method
  handle type: (MemorySegment×5)int`, making `withVisitor(...)` unusable — even a no-op visitor
  threw before any callback ran. The `JAVA_LONG` parameter layouts for genuine i64 arguments
  (e.g. `depth`, `index_in_parent`) are unchanged. Mirrors the `JAVA_INT` return layout the
  lifecycle/JSON-convention trait-bridge stubs already use.

## [0.34.3] - 2026-07-09

### Fixed

- **magnus (Ruby) codegen**: a non-variadic, infallible, synchronous free function whose
  parameters require fallible serde deserialization — a non-opaque `Named`, `Vec<Named>`, or
  sanitized `Vec<String>` param — now emits a `Result`-returning wrapper that `Ok(...)`-wraps
  the core call, instead of a stub whose `?`-based argument conversion failed to compile in a
  non-`Result` body (`E0277`). Surfaced by `max_sim_score(&MultiVectorEmbedding,
  &MultiVectorEmbedding) -> f64` and `max_sim_rank(...) -> Vec<LateInteractionMatch>`. Scoped
  strictly to this previously-broken case: variadic / error-returning / async functions keep
  their existing codegen path unchanged.
- **rustler (Elixir) codegen**: same-named NIF entries — a real definition plus its crate-root
  re-export under a narrower `cfg` (e.g. `max_sim_score`, gated `any(presets, late-interaction)`
  in its module and re-exported under `presets`) — are now collapsed via
  `dedup_same_name_functions` before re-gating. Emitting both produced two same-named
  `#[rustler::nif]` items whose cfgs overlap, which rustler auto-discovers and rejects at
  `on_load` with "Duplicate NIF entry". The other single-surface and Rust-cfg-gated backends
  already deduplicated; the native NIF generator was the last to only re-gate.

## [0.34.2] - 2026-07-08

### Fixed

- **dart scaffold**: the generated `.pubignore` now excludes native library binaries
  (`*.so`, `*.dylib`, `*.dll`) in addition to `lib/src/native/`. The FRB build stages the
  compiled library (every platform in CI) into `lib/src/<module>_bridge_generated/`, which
  is not covered by the `lib/src/native/` rule and pushed the published archive past
  pub.dev's 100MB cap (269MB observed). Native binaries are fetched at install time by
  `bin/download_libs.dart`, so none belong in the pub archive.
- **swift e2e codegen**: `count_min` / `count_equals` assertions on a scalar-string leaf no
  longer emit `.toString()?.count`. `.toString()` yields a non-optional Swift `String`, so
  optional-chaining `?.count` onto it failed to compile ("cannot use optional chaining on
  non-optional value of type 'String'"); such targets now take `.count` directly.

## [0.34.1] - 2026-07-08

### Fixed

- **codegen**: generated binding→core struct conversions now survive additive core
  changes. Every public-field `From<Binding> for Core` literal (and the lossy
  method-body and mirror-crate constructor literals in the magnus, php, dart, and
  swift backends) ends with `..Default::default()` whenever the core type
  implements `Default` — previously the trailer was emitted only when a field was
  skipped at generation time. A field added to a core config struct after
  generation now falls back to its core default instead of breaking every
  generated binding except napi with `E0063: missing field`, until the bindings
  are regenerated. Currently-mapped fields are still assigned explicitly, so
  existing conversions behave identically. `CODEGEN_FORMAT_VERSION` is bumped to
  `2` so `alef verify` re-stamps existing bindings with the forward-compatible
  literals.

## [0.34.0] - 2026-07-07

### Fixed

- **verify**: stop reporting every binding stale after unrelated changes. The inputs hash
  (`compute_inputs_hash`) no longer folds in the alef crate version (`ALEF_REV`) — a dedicated
  `CODEGEN_FORMAT_VERSION`, bumped only on output-affecting codegen changes, replaces it — and it
  now hashes a canonical, normalized serialization of `alef.toml` rather than its raw bytes. As a
  result, crate version bumps, comment/whitespace/key-order edits, and CRLF/LF differences no longer
  invalidate freshness. Source paths are normalized (repo-relative, forward-slash) before hashing.
  Adds `alef verify --verbose`, which prints the computed vs. embedded hash for each stale file.
- **scaffold (dart)**: emit `packages/dart/.pubignore` excluding bundled native libraries and
  development directories (`android/`, `ios/`, `blobs/`, `lib/src/native/`, `rust/`, `example/`,
  `test/`), so `dart pub publish` stays under pub.dev's 100 MB archive limit. The runtime
  `download_libs` script fetches the correct platform library from the GitHub release at install time.
- **e2e (swift)**: bind Vec-of-opaque accessors to a local before indexing
  (`let _vec = result.results(); _vec[0].tables()`) to prevent a use-after-free crash when
  swift-bridge releases the parent `RustVec` temporary mid-expression.
- **e2e (swift)**: emit `<expr>.toString().count` for scalar and optional-chain String
  count/emptiness assertions (previously skipped), parenthesize the optional form as
  `(… .count ?? 0)`, and bind `let result =` for `not_error` contract fixtures.

### Changed

- **rustler**: the generated `native.ex` `nif_versions` list is now driven by
  `[crates.publish.languages.elixir].nif_versions` (previously a hardcoded `["2.16", "2.17"]`),
  keeping the RustlerPrecompiled declaration in lockstep with packaging and the CI build matrix.

## [0.33.0] - 2026-07-07

### Changed

- **docs**: emit deprecation notices as Starlight-compatible `:::caution[…]` asides
  instead of mkdocs-Material `!!! warning "…"` admonitions, so generated reference
  pages render correctly under Astro Starlight. Reference pages stay `.md` (no other
  mkdocs-only syntax is generated), so type signatures with `<`, `{`, `[` need no
  MDX escaping.

### Fixed

- **docs (cli)**: expand `#[command(flatten)]` args in struct-like enum-variant
  commands. The CLI-doc generator handled `flatten` only on struct-derived commands,
  so subcommands defined as enum variants (e.g. a CLI whose `extract`/`batch` variants
  flatten an `ExtractionOverrides` args struct) emitted an opaque struct row instead of
  the flattened flags. A shared `process_command_field` helper now expands flattened
  args inline on both the struct and enum-variant paths.

## [0.32.11] - 2026-07-07

### Fixed

- **scaffold**: the generated repo-root `poly.toml` now emits the `[hooks.builtin]`
  keys `lint`/`fmt` instead of `polylint`/`polyfmt`, matching the current poly
  config schema. 0.32.10 fixed alef's own committed `poly.toml`, but the generator
  still emitted the old keys, so every downstream regen (e.g. xberg) reverted the
  config to a form poly rejects (`unknown field 'polyfmt'`). Fixed the emitter in
  `scaffold::languages::poly` and its tests.

## [0.32.10] - 2026-07-07

### Fixed

- **config**: rename the `[hooks.builtin]` keys `polylint`/`polyfmt` to `lint`/`fmt`
  in `poly.toml` to match the current poly config schema. The old keys made poly
  fail to load its config (`unknown field 'polyfmt'`), which broke the
  `poly-validate` CI job.
- **zig**: correct the trait-bridge complex-return test to assert the pass-through
  path. In the Zig trait-bridge ABI every complex return (`Bytes`, `Vec<T>`, struct,
  enum, Map) is a pre-serialized JSON `[*c]const u8` that the host impl returns
  directly, so the fallible thunk hands it back via `@constCast` rather than
  re-serializing with `std.json.fmt` (which zig 0.16 cannot apply to
  `[*c]const u8`). The codegen (shipped in 0.32.9) was already correct; the test
  still asserted the old `std.json.fmt` path. Test-only change, no codegen change.

## [0.32.6] - 2026-07-05

### Fixed

- **dart**: mirror→core conversions of `Vec<primitive>` fields now emit
  `.collect::<Vec<_>>()` instead of a bare `.collect()`. In a core struct literal
  that ends with `..Default::default()`, the field's expected type does not
  propagate through `.collect()` to pin the `x as _` cast target, so rustc
  reported `error[E0282]: type annotations needed` (e.g. `crawlberg::CrawlConfig`
  `retry_codes: Vec<u16>` from mirror `Vec<i64>`). Turbofishing resolves the
  `FromIterator` target eagerly so the element type is inferred. Applied to the
  single- and nested-`Vec` struct-field arms and the enum-variant field arm,
  matching the core→mirror direction.

## [0.32.5] - 2026-07-05

### Changed

- **java**: scaffolded Maven packages no longer wire the Spotless Maven plugin
  or emit `eclipse-formatter.xml`; Java formatting is delegated to `poly` while
  Checkstyle remains focused on correctness checks.

### Fixed

- **rustler**: plugin trait registration stubs now include the
  `implemented_methods` parameter, matching the native Rust NIF signature and
  avoiding load-time arity failures.
- **kotlin-android**: generated JNI dispatchers are public so public native
  registration methods do not expose an internal parameter type or trigger JVM
  symbol name mangling.
- **swift-e2e**: `count_min` assertions over opaque scalar method-call fields
  now convert `RustString` values to Swift `String` before checking `.count`.
- **zig-e2e**: generated tests convert returned C string pointers with
  `std.mem.span()` before JSON parsing, formatting, or byte-length assertions.

## [0.32.4] - 2026-07-05

### Added

- **php**: `package_entry_filenames` now resolves the PHP public facade class
  file (`<ExtensionNamePascal>.php`, emitted in the public-API pass) so an
  extension's `public_api_additions` attaches to it, matching the existing
  Python/Ruby wiring. Go/Dart/Node emit their entry file in a different pass and
  remain a documented no-op.

### Fixed

- **trait-bridge**: sync infallible bridge methods no longer swallow host
  failures silently. A raised/thrown host callback is logged with the wrapper
  and method name before the default value is substituted (value-returning
  methods) or the call is discarded (unit methods), so a fabricated default —
  e.g. a zero token count that reads as "fits any budget" — is no longer
  indistinguishable from a real result. Covers pyo3, napi, magnus, php, wasm
  (console.error), jni (including the host-error envelope text the dispatcher
  already marshals), rustler, extendr, the csharp primitive-return adapter,
  and the ffi null-slot/null-result edge defaults.
- **go**: generated cgo trampolines recover host panics instead of crashing
  the process, logging to stderr and returning the zero value (fallible slots
  marshal the panic text through `outError`). The invalid-handle paths —
  including the four plugin lifecycle slots — log and marshal `outError`
  instead of fabricating `1` as a return value.
- **dart**: the block_on shim logs and returns the default when an infallible
  host callback panics, instead of aborting the calling thread via `expect`.
- **java**: sync infallible trait methods now match the vtable slot signature
  exactly. Primitive/unit returns use the direct-value convention (the previous
  JSON-convention upcall stubs mismatched the C slot — a wild pointer write
  plus the status code read back as the return value — breaking such methods on
  every call); infallible `Char`/`Path` slots no longer declare a phantom
  `outError`, and infallible `Optional<non-primitive>`/`Bytes` slots declare no
  out-pointers at all, mirroring `c_return_convention`.

## [0.32.2] - 2026-07-04

### Fixed

- **swift**: first-class DTO method wrappers (`{type}_{method}_from_json`) now
  honor owned and optional parameters. Optional params are declared as
  `Option<T>` in the wrapper signature (mirroring the extern block's
  `!needs_json_bridge` guard) instead of a bare `T`, and `String`/`Named` call
  args are borrowed only when the core parameter is a reference (`is_ref`).
  Methods taking owned `String` or `Option<T>` params (e.g.
  `Response::set_cookie` / `set_header`) previously failed to compile (E0308).

## [0.32.1] - 2026-07-04

### Fixed

- **napi**: async JS handlers are now awaited in the generated handler bridge.
  The threadsafe-function return type is `Either<Promise<HandlerReturn>,
  HandlerReturn>`, so a handler that returns a thenable routes to the `Promise`
  arm (awaited on the Rust side) and a plain object routes to the value arm —
  supporting both sync and async handlers. Previously a Promise return
  serialized to `{}` and dispatch failed with a missing-field error. Adds a
  `HandlerReturn` newtype implementing `ValidateNapiValue`/`TypeName`, because
  `serde_json::Value` cannot satisfy the `Either`/`Promise` bounds directly.
- **jni**: the generated handler-bridge struct and trait-object storage now use
  `jni::refs::Global<jni::objects::JObject<'static>>` instead of the
  `jni::objects::GlobalRef` alias. In jni 0.22.4 `GlobalRef` and the whole
  `jni::objects::*` reference-type re-export are `#[deprecated]`, so the old
  emission tripped deprecation errors under `-D warnings` in generated bindings.
- **swift**: DTO Unit-returning method wrappers no longer bind `let __value =
  ...?` when the ok type is `()`, which tripped `clippy::let_unit_value` in
  generated bindings under `-D warnings`.
- **pyo3** (#174): `.pyi` stub field annotations that shadow a builtin (e.g. a
  field named `bytes`) are now qualified as `builtins.bytes` for both the field
  and `__init__` signatures, and `gen_stubs` auto-imports `builtins` — fixing a
  `mypy --strict` `valid-type` error. Salvages the #173 regression test onto
  main (the `binding_fields` converter filter has been present since 0.31.0).

## [0.32.0] - 2026-07-04

### Added

- **pipeline**: `transform_scaffold_files` extension hook, letting extensions
  post-process generated scaffold files before they are written.

### Fixed

- **jni**: trait-bridge registration now dispatches. The kotlin-android bridge
  object wraps the host in a generated `<Trait>JniDispatcher` (suspend
  interface methods are bridged via `runBlocking`), and the generated Rust
  bridge routes every trait method through its JSON `dispatch` entry point —
  previously registration discarded the object and no plugin call ever reached
  the host. Rust-defaulted methods and the `Plugin` lifecycle hooks get the
  same presence-guarded forwarding as the other dynamic backends (#170).
- **swift**: first-class DTO instance methods now emit real dispatch instead of
  being excluded/crashing. The Swift side serializes `self`, calls a generated
  Rust wrapper extern, and decodes the JSON result; the Rust wrapper
  deserializes into the **core** type (not the serde-less swift-bridge wrapper
  newtype), converts `Path` params to `PathBuf`/`&Path`, and uses swift-bridge's
  unlabeled arguments + `RustString` return. Both the extern block and the Rust
  wrapper are emitted for non-opaque types (previously nested in the `is_opaque`
  branch, so the Swift calls referenced Rust wrappers that were never generated).
  Also fixes `Renderer` trait-bridge dispatch.
- **zig**: complex trait-vtable return types are serialized to JSON and handed
  back as a caller-owned, NUL-terminated C string via `out_result`, replacing a
  placeholder that silently wrote null. Uses the Zig 0.16 `std.json.fmt` API.
- **csharp**: `Register{Trait}(impl)` now delegates to `Register`, which calls
  the native `Register{Trait}` — previously it stored the bridge but never
  registered it natively (a silent no-op).
- **rustler**: opaque resources are stored behind `Arc<RwLock<T>>` so `&mut self`
  methods (e.g. `Registry::extend_from_dir`) mutate the held value in place
  through a write lock instead of returning `Not implemented` (or, worse,
  mutating a throwaway clone). Reads take a read lock; all lock acquisitions
  recover from poison (`unwrap_or_else(|e| e.into_inner())`) to avoid crashing
  the BEAM.
- **napi**: TypeScript service wrappers call the `native{UpperCamel}` methods the
  Rust `#[napi]` glue actually exposes (`nativeRun`/`nativeIntoRouter`), not the
  bare `run`/`intoRouter` which do not exist on the native class.

### Removed

- **pyo3**: dropped the never-rendered `trait_bridge/bridge_function.jinja`
  placeholder template and its registration.

## [0.31.2] - 2026-07-04

### Fixed

- **pyo3**: field-less `_from_native_*` options converters (types whose fields
  are all binding-excluded, e.g. `App`, `GraphQLRouteConfig`) now name their
  parameter `_native` and emit a bare `return X()`, so the unused parameter no
  longer trips ruff `ARG001` in the generated `options.py`.
- **pyo3**: the visitor `Protocol` stub's "Optional methods…" note is now gated
  on `emit_docstrings`, so the default no longer emits a docstring into the
  generated `.pyi` (ruff `PYI021`/`PYI013`).

## [0.31.1] - 2026-07-04

### Fixed

- **jni**: complete the `needless_borrows_for_generic_args` fix from 0.31.0.
  The 0.31.0 change only touched the inline Optional-JSON marshaller; the
  `string_to_jstring(env, &s)` warnings in generated shims actually originate
  in the return templates. Pass the owned `String` by value there too
  (`return_optional_string`, `return_json`, `streaming_shims`).

## [0.31.0] - 2026-07-04

### Added

- **config**: `[workspace.poly.pyrefly-sub-configs]` — a glob → error-code map
  emitted as extra `[[tool.pyrefly.sub-config]]` blocks in the generated
  `pyproject.toml` (alongside the built-in `api.py` block), so extensions can
  suppress type-checker errors on generated modules whose runtime-reconciled
  pyo3 boundaries a static checker cannot follow.

### Fixed

- **pyo3**: `_from_native_*` options converters now reference only the fields
  the `@dataclass` declares (via `binding_fields`), no longer passing
  binding-excluded fields (`methods_joined_cache`, `headers_joined_cache`,
  `lifecycle_hooks`, `di_container`, …) as keyword arguments — which raised
  `unexpected-keyword` at type-check time and `TypeError` at runtime.
- **codegen**: extra clippy allows (`[workspace] extra_clippy_allows`) are now
  filtered against the backend's default allow block emitted above them, so a
  lint that is already allowed is not re-emitted — clearing clippy's
  `duplicated_attributes` lint under `-D warnings`.
- **codegen**: `clippy::redundant_field_names` is now in the crate-level allow
  block of the php, pyo3, napi, wasm, and dart backends, silencing pre-existing
  warnings in generated binding crates under clippy 1.95.
- **jni**: the `Optional` return marshaller no longer borrows the owned
  serialized `String` when calling `string_to_jstring` (`&s` → `s`), clearing a
  `clippy::needless_borrows_for_generic_args` warning in every generated JNI
  shim.
- **ffi**: the generated `build.rs` capsule header fixup now emits direct
  `header.replace(...)` statements instead of a `for` loop over an array
  literal, clearing a `clippy::single_element_loop` warning when a crate
  exposes a single capsule pointee type.
- **pyo3**: `options.py` now imports `Any` whenever `_from_native_*` converters
  are emitted (their `native: Any` parameter), not only when a `TypeRef::Json`
  field is present, fixing an `unknown-name` type-check error.

## [0.30.19] - 2026-07-04

### Fixed

- **swift**: `Vec<opaque-handle>` getters on an opaque parent type now bridge as
  a real `Vec<T>` (e.g. `ExtractionResult.results()` yields
  `RustVec<ExtractedDocument>`) instead of `Vec<String>`, so opaque-element
  accessors such as `.mimeType()`/`.content()` resolve. JSON degradation of a
  `Vec<Named>` getter to `Vec<String>` is now gated on the containing type being
  a first-class Codable struct rather than on the element type, keeping the two
  code paths (`gen_bindings` DTO classification and `gen_rust_crate` extern/getter
  emission) in lockstep via a shared `compute_first_class_dto_names` helper.
- **trait-bridge**: dynamic-backend bridges (pyo3, magnus, php, napi, wasm,
  rustler, extendr) now forward Rust-defaulted trait methods to the host
  object when it implements them, falling back to the genuine Rust default
  body otherwise. Previously a host implementation of a defaulted method
  (e.g. `supports_table_detection`, `process_document`) was silently ignored
  and the Rust default always won (#167).
- **trait-bridge**: generated host surfaces (Python `Protocol`, Ruby `.rbs`,
  PHP `interface`, Elixir behaviour, Node `.d.ts`) now match the runtime
  contract: Rust-defaulted methods are no longer required members (documented
  as optional instead), Elixir behaviours gain `@optional_callbacks` plus the
  lifecycle callbacks, and Node plugin interfaces declare the optional
  lifecycle hooks. Bridges treat a missing `initialize`/`shutdown` as a no-op
  instead of failing registration. On magnus the bridge no longer invokes
  `initialize` — which is the Ruby constructor — on host objects (#166).
- **pyo3**: plugin `Protocol` config parameters are now typed as the public
  options dataclass the package exports, and the bridge passes that type to
  the host, so an implementer typed against the public API conforms to the
  Protocol (#165).
- **rustler**: behaviour `@callback` specs now declare natively-marshalled
  struct params as `map()` instead of the stale JSON `String.t()` (#168).

## [0.30.18] - 2026-07-03

### Added

- **extension**: `Extension::public_api_additions` is now honored for **Ruby**,
  not just Python. `package_init_filename` is generalized to
  `package_entry_filenames(language, &ResolvedCrateConfig)`, which resolves each
  language's package entry file — including dynamic conventions like Ruby's
  `lib/<gem_name_snake>.rb` — so an extension can wire its public API into the
  gem entry. Additions remain append-only with exact-line de-dup and still do
  not feed the generation-inputs hash (`alef verify` unaffected). Languages
  whose entry file is produced outside the public-API pass continue to be a
  silent no-op.
- **hooks**: `alef all`, `alef scaffold`, and `alef init` now run `poly hooks
  install` after scaffolding, wiring poly's pre-commit + commit-msg git hooks
  (polylint, polyfmt, file_safety, the `cargo` builtin — clippy / cargo-sort /
  machete / deny — and the conventional-commit hook) from the generated
  `poly.toml`. Best-effort and idempotent: a no-op when `poly` is absent or the
  target is not a git repository.

### Changed

- **format**: generated code is now formatted by the `poly` (polylint) CLI as a
  single system dependency — one `poly fmt --fix` pass replaces the previous ~19
  per-language formatter shell-outs (ruff, oxfmt, rubocop, php-cs-fixer, gofmt,
  google-java-format, ktfmt, swift-format, dart, gleam, zig, shfmt, …). poly is
  invoked as a subprocess rather than compiled in, keeping alef's build lean and
  its dependency tree unchanged; a missing `poly` binary is a best-effort no-op.
  The scaffolded `poly.toml` drives lint, format, cargo interop
  (clippy/sort/machete/deny), and the pre-commit + commit-msg hooks. A residual
  `cargo sort` still runs at generation time for workspace-excluded binding
  crates so `alef verify` stays hash-stable.

## [0.30.17] - 2026-07-03

### Fixed

- **swift**: getters returning `Vec<T>` or `Option<Vec<T>>` where `T` is a
  serde-serializable struct now JSON-decode each bridged element. The Rust
  bridge serializes such collections to `Vec<String>` (per-element JSON) or a
  single JSON `String`, but the generated swift wrapper previously emitted
  `.map { try T($0) }`, which only compiles for scalar `RustVec<RustString>`
  getters and left the binding uncompilable. It now decodes with `JSONDecoder`
  (per-element for `Vec<T>`, whole-array for `Option<Vec<T>>`). Fixes generated
  bindings for core types such as `CellChange`, `PageRange`, `PageSignals`,
  `LayoutDetection`, and `PageInfo`.

## [0.30.16] - 2026-07-03

### Added

- **extension**: new `Extension::public_api_additions(api, cfg, language)`
  hook. Extensions can now contribute raw lines to a package's public-API
  init file (e.g. Python's `__init__.py`) during public-API generation, once
  per resolved language. Returned lines are appended verbatim with exact-line
  de-duplication so re-runs are idempotent; the extension owns all language
  semantics (imports, `__all__` merges). The default implementation returns an
  empty list. The appended content does not feed the generation-inputs hash,
  so `alef verify` is unaffected.

## [0.30.15] - 2026-07-03

### Fixed

- **config**: scaffold language-specific tests (`test_scaffold_python`,
  `test_scaffold_node`, and 12 others) no longer fail after
  `feat(scaffold): emit canonical rustfmt.toml`. `rustfmt.toml` is a
  repo-level file like `poly.toml`; the `language_files` test helper now
  filters it out so file-count assertions in language-specific tests remain
  stable. The `crates/alpha/Cargo.toml` fixture in the
  `sync_versions_patches_dep_tables_on_version_change` test now includes a
  minimal `src/lib.rs` stub so `cargo update --workspace --offline` no longer
  prints a "no targets specified in the manifest" error to the test output.

- **cli**: `alef sync-versions` no longer regenerates test_apps/ and scaffold
  files by default, which was causing ~20min hangs on large repos. The command
  now only updates version fields in manifests and alef.toml; regeneration is
  the responsibility of explicit `alef generate`, `alef all`, or `task
  alef:generate` invocations. Use `--regen` flag to opt into the old behavior
  (expensive, not recommended for routine version syncs).

### Added

- **poly**: `[workspace.poly.typos]` in `alef.toml` now feeds typos
  spell-checker allowlists into the generated `poly.toml`. Declare
  `[workspace.poly.typos.extend-words]` and
  `[workspace.poly.typos.extend-identifiers]` (each a `word = "word"` table)
  to preserve repo-specific allowlists across every `alef all` regeneration.
  Previously, `alef generate` clobbered hand-edited `[lint.typos.*]` sections
  in `poly.toml`; those customisations must now live in `alef.toml` under
  `[workspace.poly.typos]` (fixes #66, enables #67).

- **config**: resolve `[[crates.source_crates]]` from the cargo registry via
  `from_registry = true`. When set, each `sources` entry is treated as relative
  to the crate's published source root (resolved through `cargo metadata`)
  instead of a workspace-relative sibling path, making regeneration hermetic in
  worktrees, CI, and fresh clones. Default (`false`) behavior is unchanged.

## [0.30.14] - 2026-07-03

### Fixed

- **swift**: fix the `ExtractedDocument.tables()` opaque-`Vec` marshaling SIGSEGV
  (called out as still-open in 0.30.13). A `Vec<Named struct>` getter on a serde
  type was emitted as an opaque `RustVec<Table>`, which swift-bridge cannot
  marshal safely — dereferencing it (e.g. `.tables().count`) crashed at runtime
  with SIGSEGV. Such getters are now bridged as a JSON `Vec<String>` (mirroring
  the existing `Vec<Named enum>` handling), yielding a countable, safely
  marshaled swift collection.

### Added

- **scaffold**: honor per-target core-dependency overrides in the scripting
  bindings (#164).

### Changed

- **style**: apply canonical poly formatting (rustfmt `max_width = 120`, taplo,
  oxc) across the jni/kotlin emitters, `deny.toml`, `renovate.json`, `.mcp.json`,
  and the e2e fixture schema.

## [0.30.13] - 2026-07-02

### Fixed

- **swift**: revert the broken Option-wrapping of non-optional JSON-bridged
  `Vec<T>` extern-block return types (introduced in 0.30.10). The wrapper
  declared `Option<String>` while the impl returned bare `String`, producing an
  E0308 type mismatch that failed every consuming swift binding's compile. The
  swift codegen now emits consistent `String`/`String`. (Does not address the
  separate `ExtractedDocument.tables()` opaque-`Vec` marshaling SIGSEGV.)

## [0.30.12] - 2026-07-02

### Added

- **scaffold**: the poly scaffold now also emits a canonical repo-root `rustfmt.toml`
  (`max_width = 120`, alef-managed). poly's Rust formatter defers to rustfmt's own
  config discovery (matching `cargo fmt`), so this pins the width both tools use;
  without it rustfmt falls back to its 100 default. Every alef-managed repo
  standardizes on 120 to match poly's global `line_length` default.

## [0.30.11] - 2026-07-02

### Added

- **config**: `[workspace] extra_clippy_allows` — a string list of additional clippy lints
  to allow in every generated Rust binding file. Entries may be bare lint names
  (`"single_match"`) or `clippy::`-prefixed (`"clippy::single_match"`); both forms are
  accepted and normalised internally. The configured lints are merged (union,
  de-duplicated; defaults first, extras appended) with each backend's built-in default
  allow-list, and a single extra `#![allow(...)]` attribute is emitted after the defaults.
  When the list is absent or empty the generated output is byte-identical to the previous
  behaviour. Affected backends: pyo3, napi, magnus, php, rustler, extendr, wasm, dart,
  swift.

  Example:

  ```toml
  [workspace]
  extra_clippy_allows = ["single_match", "collapsible_match"]
  ```

## [0.30.10] - 2026-07-02

### Fixed

- **pyo3**: exclude capsule types from `_rust`-qualified return annotations. Capsule types (both raw
  round-trip and `ConstructFrom`) resolve to a host type imported from another package (e.g.
  `tree_sitter.Parser`), not a native pyclass. Qualifying them with `_rust.` in a free function's
  return annotation produced an attribute (`_rust.Parser`) that no longer exists, raising
  `AttributeError` at import on Pythons with eager annotations (<3.14). They are now excluded from
  `return_type_names`, consistent with how they are special-cased elsewhere in api.py generation.
- **swift**: nil-safe accessor for non-optional JSON-bridged `Vec<T>` fields. Wrapping such a field in
  `Option<>` makes swift-bridge emit the nil-checked accessor, matching sibling accessors, so a null
  bridged pointer degrades gracefully instead of segfaulting. Defensive fix; the underlying
  null-pointer root cause is not yet confirmed.

### Changed

- **chore**: consolidate the typos allowlist into `poly.toml` and drop dead configs.

## [0.30.9] - 2026-07-02

### Fixed

- **codegen/ffi**: complete the service-owner forward-declaration fix from 0.30.8. The new
  `api.services` loop filtered by `exclude_types`, but a service owner is `binding_excluded` by
  construction and therefore always in that set — so the owner (`App`) was still dropped and the
  `typedef struct {PREFIX}App {PREFIX}App;` never emitted. Service owners are now forward-declared
  unconditionally (their `{PREFIX}{Service}Opaque.inner` pointer references them regardless of
  exclusion). Regression test tightened to mark the owner `binding_excluded`.

## [0.30.8] - 2026-07-02

### Fixed

- **codegen/ffi**: the C header no longer references an undeclared service-owner type. The cbindgen
  forward-declaration pass iterated `api.types`/`enums`/`errors` but not `api.services`, so a service
  owner (e.g. `App`) emitted as the opaque `inner` pointer of its `{PREFIX}{Service}Opaque` handle
  (`{PREFIX}App *inner`) had no `typedef struct {PREFIX}App {PREFIX}App;` — cbindgen then failed the
  downstream C/Go build with "unknown type name". Service owners are now forward-declared too
  (filtered by `exclude_types`). Declaring the owner in `[workspace.opaque_types]` is not required.
- **sync-versions**: three alef-emitted version sites were left at the prior version on every bump.
  - Root `Package.swift`: the `.binaryTarget` artifactbundle URL
    (`releases/download/vX.Y.Z/…`) was only updated via the `v__ALEF_SWIFT_VERSION__` placeholder,
    which is gone after the first sync — so subsequent bumps left the concrete tag stale (downstream
    `from: "X.Y.Z"` consumers fetched the wrong artifact). Now rewrites the concrete
    `releases/download/vX.Y.Z/` segment too, matching the shape `verify_versions` already checks.
  - C# `.csproj`: `<InformationalVersion>` was never rewritten (only `<Version>` was). Both are now
    bumped.
  - Ruby native (Magnus) crate `packages/ruby/ext/*/native/Cargo.toml`: the core-crate dependency
    pin (`<core> = { version = "X.Y.Z", path = "…" }`) drifted because this crate is not a workspace
    member and the workspace dep-pin pass never saw it. The pin now tracks the workspace version.

## [0.30.7] - 2026-07-02

### Fixed

- **codegen/pyo3**: `_to_rust_*` converters dropped all cfg-gated fields from the Rust constructor
  call (filter was `f.cfg.is_none()`). Feature-gated fields such as `UrlExtractionConfig.crawl`
  (gated on `any(feature = "url-ingestion", feature = "url-config-types")`) ARE compiled into the
  pyo3 `#[new]` constructor, so omitting them left them unset. Added `cfg_present_for_pyo3`
  (mirroring the `.pyi` stub's `cfg_present_for_pyo3_stub`): keep fields with no cfg or whose cfg
  resolves to present in the native pyo3 build (feature gates, `not(target_arch = "wasm32")`, or
  `any(...)` of those), while still dropping genuinely platform-specific fields.
- **maven**: pin jackson to `2.19.0`. jackson 2.20+ adopted a 2-component scheme (2.20/2.21/2.22)
  only partially on Maven Central (jackson-core/databind 2.22 and any x.y.0 return 404), breaking
  generated Java/Kotlin e2e dependency resolution. `2.19.0` is fully present across all five jackson
  artifacts.

## [0.30.6] - 2026-07-02

### Fixed

- `core_to_binding_convertible_types` false-negative: types whose only non-convertible binding
  fields are excluded from the backend surface (e.g. wasm `exclude_types`) were wrongly removed
  from the convertible set. The function now accepts `excluded_field_types: &[String]` and skips
  those fields in the predicate. All non-wasm backends pass `&[]`; the wasm backend passes its
  `exclude_types` list so structs with core-only omitted fields are correctly convertible.
- Wasm `gen_struct` emitted the delegating `impl Default` unconditionally for `has_default` types
  without checking convertibility, causing E0277 when `From<core::T>` was not generated.
  Non-convertible `has_default` wasm structs now correctly keep `#[derive(Default)]` instead.

## [0.30.5] - 2026-07-02

### Fixed

- **codegen/pyo3**: suppress delegating `Default` impl for types absent from `core_to_binding_convertible_types`. The struct generator emitted a delegating `impl Default` (calling `<core::T as Default>::default().into()`) for every `has_default` type, but `gen_from_core_to_binding` is only emitted when a type passes `can_generate_conversion`. A type with `has_default=true` whose fields include an unconvertible nested type received no `From<core::T>` impl, causing E0277 in the pyo3 backend (e.g. `ServerConfig`). Fixed by adding `emit_delegating_default_for_types: Option<&AHashSet<String>>` to `RustBindingConfig` and pre-computing the eligible set in the pyo3 backend before the type loop.
- **codegen/wasm**: apply `source_crate_remaps` inside `gen_delegating_default_impl`. When a `core_crate_override` remaps the leading crate segment (e.g. `spikard` → `spikard_http`), the delegating `Default` body used the raw `rust_path` verbatim, emitting `<spikard::ServerConfig as Default>::default().into()` instead of `<spikard_http::ServerConfig as Default>::default().into()`, causing E0433 in wasm. Fixed by calling `apply_crate_remaps` on the qualified path in `gen_delegating_default_impl` and threading `source_crate_remaps` through `RustBindingConfig`.

## [0.30.4] - 2026-07-02

### Fixed

- **defaults**: unwrap `Some(inner)` Rust defaults instead of collapsing them to `Empty`.
  `expr_to_default_value` had no `Some(...)` case in the `Expr::Call` arm, so `Option` fields with a
  `Some(literal)` default (e.g. `document_max_size: Some(50 * 1024 * 1024)`,
  `extraction_timeout_secs: Some(60)`) rendered as the type's zero value — Dart's `documentMaxSize`
  became `0`, truncating fetched documents to 0 bytes. The extractor now recurses into `Some(inner)`
  so the inner literal surfaces in synthesized default-config literals across every backend that
  emits them (dart/php/swift/…).
- **php**: map cfg-gated fields the binding keeps in the `From<binding>` conversion for core. The
  enum-tainted `From<binding>` generator unconditionally skipped every cfg-gated field, letting
  `..Default::default()` fill it. PHP keeps cfg-gated fields in the binding struct
  (`strip_cfg_fields_from_binding_struct = false`), so real values (`ExtractionConfig::keywords`,
  `UrlExtractionConfig::crawl`) were silently dropped on the PHP→core conversion. The skip is now
  gated on `strip_cfg_fields_from_binding_struct`, mirroring the standard `render.rs` path.
- **wasm**: infallible trait-bridge result conversion now returns `Option`. The `unwrap_or_default`
  branch chained `.and_then` on the `Option<String>` from `.as_string()` but the closure returned a
  `Result`, failing to compile (`E0308` expected `Option`, found `Result`; `E0425` unknown `e`). The
  closure now uses `.ok()`, fixing infallible trait methods that return enums/collections
  (`backend_type`, `processing_stage`, `supported_languages`, `dimensions`).
- **wasm**: add `--allow-multiple-definition` to the scaffolded `wasm32` rustflags.
  `wasm32-unknown-unknown` has no unified libc, so multiple C deps each ship functionally-equivalent
  libc stubs (tree-sitter's shim defines `__assert_fail`; a WASI-built Tesseract bundles
  wasi-libc `assert.o`/`atexit.o`) that `wasm-ld` rejects. The emitted `.cargo/config.toml` now
  passes first-def-wins linking, a no-op unless duplicates exist.
- **e2e/dart**: clear process-global plugin registries in `tearDownAll` to prevent a cross-isolate
  deadlock. Each Dart test file runs in its own isolate, but the Rust plugin registries are
  process-global; a file that registered a Dart-backed plugin left its `DartFnFuture` callback in the
  registry after its isolate died, and a later file's isolate deadlocked (30s timeout) invoking the
  dead callback via `block_on`. The generator now emits a `clear<Registry>()` call for each
  `register_*` backend fixture present in a file, taking the Dart e2e suite from 27 to 78 passing.

## [0.30.3] - 2026-07-01

### Changed

- **scaffold**: bump the generated e2e Java `jackson-databind` version (`JACKSON_E2E`) from
  2.18.2 to 2.22.0, matching the main jackson pin so regenerated e2e poms carry the security
  update instead of drifting from a manually-bumped dependency.
- **scaffold**: fold generated-test-code lint allowances into the emitter — `A001` and `N801`
  added to `TEST_IGNORES` (generated e2e tests take an `input` param shadowing the builtin;
  generated plugin trait-bridge stub classes aren't CapWords), and `I001` added to the
  `options.py` per-file-ignore. Consumer repos no longer need repo-specific `[workspace.poly]`
  overrides for these.

## [0.30.2] - 2026-07-01

### Added

- **config**: a `[workspace.poly]` section in `alef.toml` for repo-specific poly.toml overrides —
  extra `exclude` globs and cross-engine `per-file-ignores` that the scaffolder merges into the
  generated `poly.toml`, so repo-local lint suppressions survive regeneration.

### Changed

- **scaffold**: emit a single repo-root `poly.toml` that drives lint, format, git hooks, and
  commit-message policy, replacing `.pre-commit-config.yaml` and the per-tool config files
  (`[tool.ruff]`, `[tool.mypy]`, `phpstan.neon`, `.php-cs-fixer.dist.php`, `.lintr`, `.typos.toml`,
  `.rumdl.toml`). Python type-checking moves from mypy to pyrefly. The emitted config excludes
  Jinja templates from poly (reformatting them corrupts `{{ }}` placeholders) and carries
  generated-test-code lint allowances so regenerated e2e/test-app suites stay clean.

### Fixed

- **pyo3**: strip the Rust raw-identifier prefix in `.pyi` constructor params — PyO3 exposes a
  field declared `r#type` to Python as `type`, but the stub emitted `r#type` verbatim (invalid
  Python that ruff cannot parse). The `#[new]` signature keeps `r#` to compile.
- **pyo3**: drop the duplicate OptionsField trait-bridge parameter from the `.pyi __init__` stub.
  The field was emitted both as a regular param and as the dedicated bridge kwarg, producing a
  duplicate parameter; the stub now filters the bridge field out, mirroring `#[new]`.
- **pyo3**: drop the redundant closure when wrapping a zero-argument sync core call in
  `py.detach`. `py.detach(|| xberg::list_supported_formats())` tripped `clippy::redundant_closure`
  and failed `clippy -D warnings`; zero-arg calls now pass the function path directly
  (`py.detach(xberg::list_supported_formats)`). Calls that capture arguments keep the closure.
- **php**: generate the correct return type for `serde(default = "...")` helpers on fields whose
  core type is mirrored into a binding DTO. The helper returned the core type (e.g.
  `crawlberg::SsrfPolicy`) while the field is rendered as the crate-root mirror, so the generated
  php crate failed to compile (`expected SsrfPolicy, found crawlberg::SsrfPolicy`). The helper now
  returns the mirror and converts the core value via `.into()`.

## [0.30.1] - 2026-06-29

### Fixed

- **tests**: normalize docs-stage generated path assertions across Windows and Unix.
- **java**: always generate `ByteArraySerializer.java`. The generated ObjectMapper registers
  `new ByteArraySerializer()` unconditionally, but the class was only emitted when a record had a
  non-optional `Bytes` field — leaving a dangling reference that fails to compile for packages
  without one. It is now emitted unconditionally, matching `JsonUtil`.

## [0.30.0] - 2026-06-29

### Added

- **docs**: add a template-driven docs stage for API, CLI, MCP, `llms.txt`, agent skills, and
  snippet validation. Repos can configure generated reference output, required local templates for
  `llms.txt` and grouped skill files, static Clap/rmcp source extraction, and docs-specific snippet
  checks. Alef now warns on explicit skipped docs inputs such as missing configured sources or
  unavailable snippet toolchains while avoiding noisy warnings for unset optional docs layers.

- **snippets**: `typecheck` validation level. Ordered between `compile` and `run`, it statically
  type-checks a snippet without executing it, and for compiled languages without needing the native
  library. Each language runs its strict static checker: `python -m mypy`, `tsc --noEmit`,
  `cargo check`, `go vet`, `javac -Xlint:all -Werror`, `dotnet build -warnaserror`,
  `swiftc -typecheck -warnings-as-errors`, `kotlinc -Werror`, `dart analyze --fatal-infos`, and
  `cc -fsyntax-only -Wall -Werror`. This catches dual-representation mistakes (a config field typed
  against a flattened union alias that rejects the documented data-enum constructor) that
  `py_compile` and a lenient compile cannot see. A matching `snippet:typecheck-only` ceiling
  annotation sits alongside `syntax-only` and `compile-only`. mypy is optional: when it is not
  installed the Python snippet is reported as unavailable rather than failing.

### Fixed

- **napi**: give the generated streaming `WORKER_POOL` tokio runtime a 16 MB worker stack, so a
  deep consumer future does not overflow the default (~2 MB) worker stack and abort with `SIGBUS`.
- **pyo3**: provision an enlarged worker-thread stack on the generated module's async runtime.
  pyo3-async-runtimes' default multi-thread runtime gives workers a small (~2 MB) stack, which a
  deep consumer future (e.g. a multi-stage OCR pipeline) overflows — aborting the whole process
  with `SIGBUS`. The `#[pymodule]` init now installs a `tokio` runtime with a 16 MB
  `thread_stack_size` before the first `future_into_py`.
- **pyo3**: serialize `dict`/`list` values for JSON (`serde_json::Value`) config fields in the
  generated `api.py` converters. PyO3 cannot expose a settable `serde_json::Value` field, so the
  binding stores such fields as `str`, while the public dataclass and `.pyi` stub type them as
  `dict[str, Any]`. The converter forwarded the dict straight through, so the documented dict form
  raised `TypeError: 'dict' object is not an instance of 'str'` at runtime; it now `json.dumps`es a
  dict/list (passing `str`/`None` through unchanged).
- **pyo3**: re-point each re-exported exception's `__module__` at the public package in the
  generated `exceptions.py`. The classes are the native ones (`create_exception!` sets their
  module to the compiled `_native` extension), so tracebacks and `repr()` previously read
  `_native.DownloadError` instead of the public name, and the exceptions were not picklable under
  their public path. `exceptions.py` now reassigns `__module__` for every name in `__all__`
  (tree-sitter-language-pack issue #147).
- **codegen**: generate compiling binding→core conversions for core structs that have private
  (`pub(crate)`) fields. Such a struct cannot be built with struct-literal syntax from a foreign
  crate — neither by naming the private field nor by patching it with `..Default::default()` — so
  the conversion now seeds the core type's `Default` (which fills the private fields inside the
  defining crate) and assigns only the public fields onto it. The strategy is centralized in a
  shared helper used by the pyo3/napi/wasm/extendr/rustler/magnus generator, the Dart mirror crate
  generator, and the PHP enum-tainted conversion path; when the core type has private fields but no
  `Default`, a `compile_error!` guides the author to derive `Default`. A new `has_private_fields`
  flag on struct IR records the condition during extraction.
- **php**: marshal owned (by-value) native-struct callback parameters by value rather than
  dereferencing them as a borrow (`(*input)` does not type-check on an owned `core::T`), and stop
  emitting the native-object return fast-path — a PHP `#[php_class]` binding struct implements
  `FromZvalMut` (for `&mut T`) but not `FromZval` (for `T`), so the bridge keeps the JSON return
  path that is well-defined for PHP.
- **pyo3**: marshal owned (by-value) native-struct callback parameters into the host's native
  binding object via `From<core::T>`, the same way borrowed ones already were. A trait method that
  takes a serde struct by value (e.g. an extraction-input envelope) previously passed the raw
  `core::T` across the Python boundary, which has no `IntoPyObject` and failed to compile.
- **pyo3**: when a core `register_*` free function shares its name with a trait bridge's
  `register_fn`, emit only the bridge's duck-typed registration. The function loop no longer also
  emits the auto-wrapped core version, which collided (`E0428`) with the bridge definition and no
  longer type-checks against a registry that takes `Arc<dyn Trait>`.
- **pyo3**: the generated Python package now type-checks clean under `mypy`. Data-enum config fields
  are annotated against their public class (so `EmbeddingConfig(model=EmbeddingModelType.plugin(...))`
  is accepted) instead of a flattened union alias that shadowed the class; constructors accept the
  public dataclass/dict for factory parameters; data-enum `__init__` signatures match the runtime
  `#[new]`; `Json` maps to `dict[str, Any]`; and the duplicate `clear_*` registry stub is no longer
  emitted twice.
- **napi**: substitute binding-excluded types (e.g. `InternalDocument`) with `JsonValue` in the
  `.d.ts` host-interface signatures. Referencing a type that is never emitted produced an undefined
  TypeScript name; the runtime bridge marshals such values as JSON, so `JsonValue` is the faithful
  stand-in and `tsc --strict` is clean.
- **magnus**: apply the same excluded-type substitution (to `json_value`) in generated `.rbs`
  interfaces and skip re-declaring a bridge `clear_*` function that is already exposed as a registry
  function, so `rbs validate` no longer reports an undefined type or a duplicated method definition.

- **node/wasm**: require Node 22 or newer in generated npm package
  manifests, and keep Python package generation on Python 3.10 or newer.

- **e2e/dart**: resolve `config` JSON object helper types from compatible
  call overrides so generated tests use concrete helpers such as
  `createExtractionConfigFromJson`.

- **wasm**: filter cfg-gated struct fields with the WASM backend's active feature set so
  inactive fields are omitted and active fields are generated consistently across structs,
  constructors, accessors, and conversions.

- **r**: keep cfg-gated struct fields when the R backend's configured feature set enables
  them, and align R wrapper exports with the classes registered in `extendr_module!`.

- **scaffold**: let managed `.cargo/config.toml` render an explicit
  `rustc-wrapper`, and make the R Rust crate honor curated feature sets the
  same way as WASM by disabling core default features and declaring cfg
  passthrough features without enabling them by default.

- **r**: merge crate-level `extra_dependencies` into the generated R Rust
  crate so external DTO conversion impls can depend on sibling Rust crates
  such as `crawlberg`.

- **elixir**: render known generated public DTO fields in struct typespecs as
  their concrete module types instead of falling back to `map()`.

- **swift**: filter host Swift bindings with the same effective cfg feature set
  as the generated Rust bridge crate, including default cfg passthrough
  features.

- **swift**: wrap method-shim DTO returns for `Option<&T>` and `Vec<T>`, and
  pass `&Path` method parameters as borrowed paths instead of owned `PathBuf`s.

- **pyo3/magnus/wasm**: delegate generated binding defaults for defaultable
  DTOs to the core Rust `Default` impl so omitted nested config fields keep
  semantic core defaults.

- **extract**: support root-scoped external DTO source crates so host bindings
  can expand typed config graphs from sibling crates without exposing sibling
  functions or importing sibling language packages.

- **extract**: preserve explicit field `type_rust_path` values and reject
  same-name types from different crates, while keeping binding-excluded fields
  out of include-list expansion.

- **go/java**: avoid callback return local-name collisions in generated trait
  bridges when a method parameter is named `result`.

- **ffi**: keep cbindgen forward declarations for live binding DTOs when cfg-gated
  skipped duplicates leave older entries in Alef's excluded type-path map.

- **dart**: suppress ordinary trait-bridge lifecycle wrappers so FRB only sees the generated
  `{Trait}DartImpl` registration surface.

- **e2e**: emit typed single-call `json_object` inputs for Dart, Swift, and R so unified
  `extract(input, config)` fixtures pass their `ExtractInput` payload instead of defaulting it away.

- **pyo3**: include Pyo3-present cfg-gated fields in generated `.pyi` constructor stubs so native
  signatures and type stubs agree for typed nested configs such as `UrlExtractionConfig.crawl`.

- **dart**: normalize trailing whitespace in FRB-generated Dart files, including `*.freezed.dart`
  files that `dart format` leaves unchanged.

- **e2e**: prefer configured config DTO types when rendering Dart `config`
  JSON objects, preventing fallback helpers such as `createConfigFromJson`.

- **e2e**: include WASM nested DTO imports reached through `json_object`
  element types, such as per-input file configs nested under extract inputs.

- **elixir**: JSON-encode default-typed single DTO parameters before calling
  Rustler NIFs, matching the NIF boundary used for unified extract inputs.

## [0.29.4] - 2026-06-27

### Changed

- **tooling**: extend the `no-project-special-casing` pre-commit hook to reject the `xberg` and
  `crawlberg` downstream product names (case-insensitive, including camelCase and separator
  variants), and consolidate the brand allowlist so the `xberg-io` org namespace and the `xberg.io`
  domain stay permitted while `xberg-io/xberg` and bare `xberg` mentions are still caught. Neutralize
  the `xberg`-named Java/enum test fixtures to generic sample names.

### Fixed

- **e2e**: keep public Ruby and Elixir test calls on configured method names and
  resolve `$mock_url` placeholders inside typed JSON-array arguments across
  generated language e2e suites.

- **e2e**: resolve `$mock_url` placeholders for Ruby object arrays, Elixir typed
  object arguments, and Kotlin/PHP typed object setup while allowing Elixir e2e
  calls to target keyword-opts public facades.

- **e2e**: avoid Elixir typed-object variable collisions and align Kotlin typed
  object mock URL fallbacks with the generated mock-server harness.

- **node**: remove downstream internal DTO names from generated trait-bridge
  return-value comments.

- **ffi**: honor `[crates.ffi].exclude_types` when generating `cbindgen.toml`.
  Excluded Rust-only helper DTOs are now omitted from the header prelude forward
  declarations and emitted in `[export].exclude`, keeping C and cgo headers from
  leaking types that the FFI layer does not expose.

- **java/kotlin-android**: route configured trait-bridge lifecycle functions through the generated
  bridge APIs instead of also emitting ordinary FFI wrappers. This keeps raw Rust functions such as
  `register_document_extractor` from shadowing typed host interfaces (`IDocumentExtractor`,
  `IRenderer`) with dangling `DocumentExtractor`/`Renderer` parameter types or JSON-string JNI
  declarations.

## [0.29.3] - 2026-06-26

### Fixed

- **java/kotlin-android**: honor per-language `generate.async_wrappers = false` when emitting
  Java `CompletableFuture` helpers and Kotlin Android suspend convenience wrappers. This keeps
  bindings that want a single canonical method name from leaking extra `fooAsync` entrypoints while
  still preserving Rust functions that are themselves named `*_async`.

- **java (scaffold)**: derive the `maven-source-plugin` source include from the Maven group's first
  path segment instead of a hardcoded `dev/**`. After the `dev.kreuzberg` → `io.xberg` rebrand,
  generated sources moved to `io/<group>/…`, so the stale `dev/**` include matched nothing, the
  source jar came out empty, and Sonatype Central rejected the deployment with "Sources must be
  provided but not found in entries". The include now tracks the group (`io/**` for `io.xberg.*`).

## [0.29.2] - 2026-06-26

### Fixed

- **java**: read i32-returning FFM downcall results as `(int) (long)` instead of `(int)`. Since all
  integer FFM layouts are promoted to `JAVA_LONG` (for JBR Win64 Panama compatibility), the downcall
  handle returns `long`; casting the `invoke(...)` result straight to `(int)` forced an illegal
  `long → int` `asType` conversion that threw `WrongMethodTypeException` at the call boundary. This
  broke every byte-result method (e.g. `speech`, `fileContent`) and the trait-bridge
  register/unregister/clear lifecycle calls. The call sites now narrow via `(int) (long)`, matching
  the canonical pattern already used for `last_error_code`.

- **swift**: encode enum-typed struct field getters to match how the Swift side decodes each enum
  kind. Tagged enums (some variant carries data, e.g. `AssistantContent`) are serialized with
  `serde_json::to_string` of the source value and decoded via `JSONDecoder` — the discriminant-only
  bridge wrapper's `.to_string()` previously dropped the payload and returned an unquoted name (e.g.
  `Text`), which `JSONDecoder` rejected with "The given data was not valid JSON." Unit enums (all
  variants fieldless, e.g. `FinishReason`) keep returning their bare serde raw value via the wrapper's
  `.to_string()`, which Swift reconstructs with `Type(rawValue:)`; serializing those to JSON would
  emit a quoted string the rawValue init cannot parse.

- **elixir**: keep async NIF symbols suffixed internally while exposing async free functions under
  their original public names in the high-level Elixir facade. Generated modules now expose
  `extract/1` and `extract_batch/1` when the Rust API names are `extract` and `extract_batch`, while
  still delegating to `Native.extract_async/2` and `Native.extract_batch_async/2`.

- **magnus**: register suffixed async helper functions under their original public Ruby names. Ruby
  bindings now expose canonical methods such as `extract` and `extract_batch` even when the generated
  native helper functions are named `extract_async` and `extract_batch_async`; RBS stubs use the same
  public names.

### Removed

- **napi: stop generating the legacy `packages/typescript` wrapper package.** The napi backend no
  longer emits the `packages/typescript/src/index.ts` re-export barrel or its `bridges/*.ts` files;
  the native package (`crates/{lib}-node`, published with its own `index.d.ts`) is the canonical
  TypeScript surface, and `packages/node` is the modern package directory. `generate_public_api` for
  the napi backend now falls back to the default (no-op), and the existing orphan sweep removes any
  previously generated `packages/typescript/` tree on the next run. Version sync/checks and the e2e
  node package fallback now reference `packages/node` instead of the legacy `packages/typescript`.

### Added

- **e2e: support typed JSON-object arguments and `$mock_url` placeholders inside request DTOs.**
  Generated e2e tests now resolve non-array `json_object` argument types from per-argument metadata
  (`element_type`, and `go_type` for Go) before falling back to call-level `options_type`, so calls with
  separate request/config DTOs can be generated correctly. Structured JSON args can also embed
  `$mock_url`, which is replaced at test runtime with the fixture's mock-server URL.

- **e2e: accept fixture-level args, config, and route mocks in validation.**
  The embedded fixture schema now matches Alef's fixture model for per-fixture argument overrides,
  top-level `config`, `mock_response`, `setup`, `env`, and HTTP fixtures. Fixture loading mirrors
  top-level `config` into `input.config` before generation, and semantic missing-field validation now
  respects fixture-level `args`.

## [0.29.0] - 2026-06-26

### Fixed

- **pyo3 (Python): qualify builtin containers shadowed by a data-enum variant factory name.**
  A data enum with a `List` variant emits a `def list(...)` `@staticmethod` factory, which shadows the
  builtin `list` within the class body — so a sibling factory annotated `entries: list[MetadataEntry]`
  resolves to the factory and mypy rejects the `.pyi`
  (`Function ... is not valid as a type [valid-type]`). Factory annotations now qualify a shadowed
  builtin container (`list`/`dict`/`set`/`tuple`/`frozenset`/`type`) as `builtins.<name>[...]`, and the
  stub emits `import builtins` when referenced.

- **java: promote all integer FFM `FunctionDescriptor` layouts to `JAVA_LONG` for JBR Win64 Panama
  compat.** JetBrains Runtime's Panama linker casts every descriptor layout to `OfLong` internally, so
  any sub-64-bit integer layout (`JAVA_BYTE`/`JAVA_SHORT`/`JAVA_INT`) threw
  `ClassCastException: OfIntImpl cannot be cast to OfLong` at `NativeLib` class load and corrupted
  `TreeCursor` FFM calls. `java_ffi_type`, `service_api`, the enum-discriminant layout, the
  `LAST_ERROR_CODE` descriptor, and the visitor/trait-bridge/registration callback descriptors now
  emit `JAVA_LONG` for bool, 8/16/32-bit ints, and enum discriminants. `java_ffi_return_cast` emits
  compound narrowing casts (`(int)(long)`, `(short)(long)`, `(byte)(long)`) and the primitive-result
  templates no longer double-wrap them in parens. Generated `FunctionDescriptor`s now contain zero
  sub-64-bit integer layouts.

- **swift: add a runtime rpath to the generated `Package.swift` so the FFI dylib loads at runtime.**
  The `RustBridge` target emitted only `-L` (compile-time search). Because the FFI dylib's
  install_name is `@rpath/lib…dylib`, the consumer (and any test bundle linking the target) needs an
  `LC_RPATH` or `swift test` aborts with `dlopen … Library not loaded: @rpath/libhtml_to_markdown_ffi.dylib`.
  The manifest now derives the Cargo target dir absolutely from `#filePath` (CWD-independent, like the
  Zig/C e2e generators) and adds the rpath for both the release and debug profiles via the
  swiftc-native `-Xlinker -rpath -Xlinker <dir>` spelling (swiftc rejects `-Wl,-rpath,<dir>`). The e2e
  Swift package inherits the rpath transitively through this target.

- **extendr (R): skip per-variant factory constructors whose fields cannot cross the extendr input boundary.**
  A tagged data enum (e.g. `NodeContent`) generates a `_factory_<variant>` `#[extendr]` constructor per
  struct variant. When a variant field was a Named DTO (`grid: TableGrid`) or `Vec<DTO>`
  (`entries: Vec<MetadataEntry>`), the constructor took it *by value*, which the `#[extendr]` proc-macro
  cannot accept (`error[E0277]: T: TryFrom<&Robj> not satisfied`) — extendr derives `TryFrom<&Robj>` only
  for `&T`, never owned `T`, and has no R-list conversion for `Vec<DTO>`. `gen_extendr_enum_variant_constructors`
  and `extendr_enum_variant_constructor_registrations` now skip such variants (predicate
  `extendr_factory_param_is_constructible`); those variants remain constructible via the enum's `from_json`
  factory.

- **extendr (R): exclude methods with R-incompatible `Vec`/`Option<Vec>` params from `#[extendr]` impls.**
  Method filtering only dropped methods with bare-enum or bare owned-struct params; it missed
  `Vec<struct>`, `Vec<enum>`, `Vec<Vec<_>>`, and `Option<Vec<_>>` params. extendr generates no
  `TryFrom<&Robj>` for those, so the proc-macro failed downstream with
  `error[E0277]: T: TryFrom<&Robj> not satisfied` (e.g. `Vec<MetadataEntry>`). The two method-filter
  sites in `gen_bindings/mod.rs` now also apply the existing `is_extendr_native_incompatible` param
  check (already used for free functions), so such methods are omitted from the impl block.

- **php: per-variant constructor boxes `Box<T>` fields.** The flat-data-enum factory
  (`gen_flat_data_enum_variant_constructors`) emitted `field: field.clone().into()` for a variant
  field whose core type is `Box<T>`/`Option<Box<T>>` (Named `T`), which fails to compile (no
  `From<Binding> for Box<Core>`). It now wraps the converted value in `Box::new(...)` (or
  `.map(Box::new)` when optional), using the `VariantConstructor::boxed` flags — mirroring
  `flat_enum_binding_to_core_field_expr` and the shared `variant_field_init`.

- **magnus: per-variant constructors no longer collide with tagged-enum modules.** Tagged data enums
  are represented on the Ruby side as a `module <Name>` interface with per-variant `Data.define`
  classes, but the per-variant-constructor feature also emitted a Rust `module.define_class("<Name>")`
  with singleton factories. At load the `.so` defined the class first, so the pure-Ruby `module <Name>`
  raised `TypeError: <Name> is not a module` and the extension failed to load. Tagged data enums now
  skip the Rust factory class entirely — the class/singleton registration (`module_init`), the Rust
  `_factory_*` methods (avoids unused-method `-D warnings`), and the `.rbs` singleton stubs are all
  gated on `serde_tag.is_none()`. Construction for tagged enums goes through the variant `Data` classes
  (`<Name>Basic.new(...)`) and `from_hash`; non-tagged data enums keep their factory constructors.

### Added

- **Exception handling architecture guide and cross-language pattern documentation.** Added comprehensive
  `EXCEPTION_HANDLING.md` documenting exception/error handling patterns across all 15 language bindings
  (Python, Node.js, Ruby, PHP, Go, Java, C#, Elixir, WebAssembly, Dart, Swift, Kotlin Android, R, Zig, C FFI).
  Covers issue #147 (Python exception class identity), type identity preservation, error code standardization
  (1000+), and implementation checklists for new bindings. Ensures consistency across polyglot bindings.

- **CI resource optimization guide.** Added `CI_RESOURCE_OPTIMIZATION.md` documenting optimization strategies
  for large polyglot codebases (300+ grammars) on resource-constrained GitHub-hosted runners. Covers concurrency
  tuning (CLONE_CONCURRENCY=8, GENERATE_CONCURRENCY=2), sharding across parallel jobs, memory monitoring,
  and troubleshooting. Resolves exit-code 143 (SIGTERM) resource exhaustion issues.

- **PyO3 exception handling pattern documentation.** Enhanced `src/backends/pyo3/gen_bindings/errors.rs` with
  detailed cross-language exception handling patterns and core principle that exception class/type identity
  raised by native code must match the type exposed by public API. Reference for all polyglot backends.

### Trait-callback host returns accept the native binding object across the dynamic backends

  (pyo3, magnus, php, extendr).** Host-implementable trait callbacks already received native
  arguments (#142/#143), but the return value was still marshalled through a mapping/JSON path that
  rejected the binding's native result object even though the generated host interface advertised
  that type. Each dynamic backend's return path now tries the native object first
  (`extract::<Binding>()` / `TryConvert` / `FromZval` / `ExternalPtr` unwrap) and converts via
  `From<Binding> for Core`, falling back to the existing dict/array/hash/JSON path. The native path
  is gated on the binding→core conversion actually being generated (`convertible_types`), and extendr
  additionally gates on extendr-representability so non-representable rich types keep the JSON path. A
  shared `native_marshalled_struct_returns` classifier mirrors the param-side allowlist. On pyo3 the
  Protocol method also changes from `async def` to `def`, matching the `spawn_blocking` bridge that
  never awaited it. Resolves #153.

### Fixed

- **Per-variant constructors now box `Box<T>` fields.** When a data enum's struct variant has a
  field whose core type is `Box<T>`/`Option<Box<T>>` for a Named `T` (e.g. `CrawlEvent::Page {
  result: Box<CrawlPageResult> }`), the generated `_factory_<variant>` constructor emitted
  `result.into()`, which fails to compile because there is no `From<Binding> for Box<Core>`. The
  factory path now wraps the converted value (`Box::new(result.into())`, or
  `result.map(Into::into).map(Box::new)` for the optional case), mirroring the existing
  `From`/`Into` impl path (`conversions::binding_to_core::render`). The `is_boxed` flag is carried
  on `VariantConstructor` (parallel to `params`) and threaded into `variant_field_init`, so the
  pyo3, magnus, and extendr per-variant factories all box correctly.
- **pyo3 (Python): type stubs declare per-variant data-enum constructors.** The `.pyi` stub for a
  tagged data enum now emits a `@staticmethod` per data-carrying variant — `def circle(radius: float)
  -> Shape: ...` — between the tag attribute and the `__str__`/`__repr__` dunders, so type-checkers and
  IDE autocomplete see the `Shape.circle(...)` factories the runtime binding already exposed. The
  declared name is the public host name (`#[pyo3(name = "<snake>")]`), each param maps through the
  stub's `python_type` mapper, and the return type is the enum. Optional params — naturally optional
  fields and those promoted because they follow an optional one — render as `T | None = None`, matching
  the runtime constructor signature. Variant selection is shared with the runtime binding via
  `collect_variant_constructors`, so unit / tuple / `binding_excluded` / sanitized-field variants and
  hand-written method collisions are skipped identically.
- **magnus (Ruby): RBS stubs declare per-variant data-enum constructors.** The `.rbs` stub for a
  tagged data enum was an empty `class Shape ... end`; it now declares a singleton method per
  data-carrying variant — `def self.circle: (Float radius) -> Shape` — so RBS sees the
  `Shape.circle(...)` factories the runtime binding registers via `define_singleton_method`. The
  declared name is the bare snake_case host name, each param maps through the stub's `rbs_type`
  mapper, and the return type is the enum. Optional params — naturally optional fields and those
  promoted because they follow an optional one — render as the nilable `?T name` form, matching the
  runtime constructor signature. Variant selection is shared with the runtime binding via
  `collect_variant_constructors`, so unit / tuple / `binding_excluded` / sanitized-field variants and
  hand-written method collisions are skipped identically.
- **php: type stubs declare per-variant data-enum constructors.** The IDE/PHPStan stub for a tagged
  data enum (lowered to a flat PHP class) was an empty `final class Shape {}`; it now declares a
  static factory per data-carrying variant — `public static function circle(float $radius): Shape` —
  so PHPStan and IDEs see the `Shape::circle(...)` constructors the flat class exposes at runtime. The
  declared name is the camelCase host name (`to_php_name`), each param maps through the stub's
  `php_type` mapper (optional fields become `?T $x = null`), and the return type is the enum class.
  Variant selection is shared with the runtime binding via `collect_variant_constructors`, so unit /
  tuple / `binding_excluded` / sanitized-field variants and hand-written method collisions are skipped
  identically.
- **pyo3 (Python): enum-variant payloads accept the public dataclass/dict.** A data-enum
  per-variant constructor (e.g. `EmbeddingModelType.llm(...)`) now coerces a config-DTO payload the
  same way struct fields are coerced, so passing the public `LlmConfig` dataclass — or a plain
  `dict` — builds the variant instead of raising `TypeError: 'LlmConfig' object is not an instance
  of 'LlmConfig'`. Previously the generated factory demanded the compiled `#[pyclass]` instance
  while the package re-exported the pure-Python `@dataclass` for the same name, so the two never
  matched. A payload field whose type is a dataclass-backed config DTO — directly, or as a
  `list`/`dict`/`Optional` of one — is now generated as `&Bound<PyAny>` and routed through the
  module-level `__alef_coerce_dto` helpers (dataclass via `dataclasses.asdict` / dict / JSON-native
  → serde into the core type). Renamed fields round-trip with full fidelity: a per-DTO
  `__ALEF_WIRE_*` schema rewrites dataclass field names to serde wire names, honoring both
  `#[serde(rename)]` and `#[serde(rename_all)]` and recursing through nested DTOs, sequences, maps,
  and optionals — wire names are sourced from the same centralized naming transform the Python
  `_to_rust_*` converters use. Native re-exported return types stay compiled and are left untouched;
  the config-vs-native-return classification is shared with `__init__.py` import routing as a single
  source of truth (xberg #1165).

## [0.1.0 – 0.28.1] - 2026-04-09 – 2026-06-25

Early development history (592 releases through 0.28.1) has been trimmed to keep
this file small. The full per-version changelog is preserved in the git tags and
GitHub releases: <https://github.com/xberg-io/alef/releases>
