# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`jni` no longer hard-fails generation when `[crates.kotlin_android]` is unconfigured.**
  Every downstream accessor (`jni_kotlin_package`, `jni_excluded_functions`,
  `jni_excluded_types`, `jni_capsule_types`) already tolerated its absence, falling back to
  the same vendor-neutral placeholder package `kotlin`/`java` use when unconfigured — the
  `generate_bindings` guard was the only place still bailing on a config gap every sibling
  accessor already treated as a soft default, so enabling `jni` without also configuring
  `kotlin_android` produced a hard generate failure for a language the consumer did
  configure.

## [0.62.5] - 2026-08-20

### Added

- **`[crates.test.<lang>].e2e_precondition`** lets a block scope the `e2e` tooling gate
  separately from the block's main `precondition`. A block with only `before` + `e2e` (no
  `command`) previously had to satisfy validation by writing a `precondition` for whatever
  `command`/`before` needed, and `alef test --e2e` then gated `e2e` on that same, often
  unrelated, check.

### Fixed

- **The scaffolded Maven `attach-javadocs` execution no longer fails for any consumer that has
  Java tests.** The pom sets `<sourcepath>${project.basedir}</sourcepath>` because alef emits a
  flat source layout with no `src/main/java/`, but that also pointed javadoc at `src/test/java/`,
  whose JUnit/AssertJ imports are test-scoped and absent from the javadoc classpath. Combined with
  the `failOnWarning` the same pom sets, `mvn package` died with hundreds of
  `package org.junit.jupiter.api does not exist` errors. maven-source-plugin already restricted
  itself to the publishable subtrees for the same underlying reason; javadoc was the one plugin
  left unrestricted, and it now carries the matching `<sourceFileIncludes>`.

- **`alef test --lang <X> --e2e` no longer skips the e2e suite on a precondition written for
  the block's `command`, not for `e2e`.** `e2e` is now gated by the new `e2e_precondition` when
  set; when unset, `e2e` runs ungated instead of inheriting the main `precondition` (which was
  authored for a different command and could name tooling `e2e` never uses, e.g. a linter). The
  main `precondition` still gates `command`/`coverage` exactly as before. `before` is unchanged
  and still runs ahead of `e2e`, since it commonly builds the native library the e2e suite loads.

- **E2e enum-field detection is now derived from the IR, not only from a hand-written
  `alef.toml` `fields_enum` list.** `E2eConfig::effective_fields_enum` returned purely
  author-declared sets, so a consumer that never enumerated its enum fields got `false` for
  every one of them, and the Rust generator emitted `<field>.to_string()` for an enum-typed
  field -- a compile error (`E0599: doesn't implement std::fmt::Display`) for any enum that
  only derives `Debug`. `FieldResolver::is_enum` now falls back to a new IR-derived
  classification (`FieldResolver::ir_enum_fields` / `with_ir_enum_map`, in the new
  `e2e::field_access::ir_enum` module) that walks a field path from the call's declared Rust
  result type -- resolved from the crate's own function/method signatures, not a
  per-language override -- through `Option`/`Vec`/`Box`-wrapped and array-traversed
  (`links[].link_type`, `choices[0].finish_reason`) paths to the exact struct that owns the
  leaf field, and checks whether its declared type is a real IR enum. The classification is
  keyed by `(owner type, field name)`, so a field name that means different things on
  different types (`kind: String` on one struct, `kind: SomeEnum` on another) is never
  conflated. An explicit `fields_enum` config entry still wins over the IR when both apply.

## [0.62.4] - 2026-08-20

### Fixed

- **The Rust e2e generator no longer emits `.to_string()` on enum-typed fields, which does
  not compile unless the enum happens to derive/implement `Display`.** An `equals` assertion
  on an enum field (e.g. `kind: DataNodeKind`) rendered
  `result.kind.to_string()`, but alef requires no such trait -- most bound enums only derive
  `Debug`. `render_equals_assertion` already had `field_is_enum` plumbed through
  `FieldResolver::is_enum` for containment assertions, but never consulted it for `equals`, so
  every enum-field equals assertion failed to compile (`error[E0599]: doesn't implement
  std::fmt::Display`). It, and the analogous wildcard array-traversal `contains`/`not_empty`
  predicates (`links[].link_type`), now stringify enum-typed leaves via `format!("{:?}", ...)`
  (Debug), matching what the existing containment predicate already does -- for a unit variant
  this renders exactly the variant name, matching the fixture's captured expected literal.

- **A backfilled cfg-forwarded feature is now also enabled by default, not just declared.**
  Declaring `<feature> = ["<core-crate>/<feature>"]` in the Ruby/Elixir native manifest's
  `[features]` table does not turn the feature on -- `#[cfg(feature = "X")]` stayed false, and
  the affected definitions kept silently compiling out, even after the previous repair pass added
  the forwarding row, because nothing added `X` to `default` and no build wrapper alef scaffolds
  passes `--features`. `merge_missing_cfg_features` now also appends any referenced feature
  missing from `default` (whether newly declared or already declared but never defaulted),
  mirroring what `scaffold_ruby_cargo`/`scaffold_elixir_cargo` already write on a fresh scaffold.
  `warn_on_undeclared_binding_cfg_features` now keys on `read_default_enabled_cargo_features`
  (reachable from `default`) instead of mere declaration, so a feature that is declared but not
  defaulted still warns instead of reading as fixed.

- **`alef scaffold` now actually adds a cfg-forwarded feature the compile-out warning names,
  instead of leaving the prescribed remedy a no-op.** The Ruby (Magnus) and Elixir (Rustler)
  native manifests are `generated_header: true`, so a full regen already includes every feature
  `collect_cfg_features` finds — but once the manifest exists on disk,
  `write_scaffold_files_report`'s ownership guard only overwrites it wholesale when it can prove
  alef authored the existing bytes, and a manifest predating the marker scheme (or one whose
  marker a hand-edit or formatter moved past the guard's scan window) is refused forever, so
  "re-run `alef scaffold`" never converged. `alef scaffold` and `alef generate` now also run a
  narrower, always-safe repair: it inserts only the missing `<feature> =
  ["<core-crate>/<feature>"]` row(s) into the manifest's `[features]` table (creating the table
  if absent) via `toml_edit`, which cannot reorder, reformat, or drop anything else already in
  the file, and never invents a row for a feature the core crate itself does not declare.

## [0.62.3] - 2026-08-19

### Fixed

- **The serde-default-disagreement warning no longer fires when both defaults are the same
  zero value spelled differently.** A bare `#[serde(default)]` always folds to
  `DefaultValue::Empty`, but a hand-written `impl Default` that spells the zero out explicitly
  (`count: 0`, `enabled: false`, `label: String::new()`, `handle: None`) folds to a literal
  instead. `warn_on_default_disagreement` compared the two spellings structurally and reported a
  divergence that did not exist; it now treats `Empty` and its type-zero literal counterparts as
  equal before deciding whether to warn.

- **The disk-scan orphan report no longer asserts a file "was not emitted".** What it can actually
  observe is that a path is absent from the run's recorded output, and the check immediately above
  it warns that some backends record nothing beyond their Rust crate path — so non-emission is one
  of four explanations, not a fact. The report now says what it knows.

- **e2e fixture diagnostics are logged at the severity they carry.** Both arms of the severity
  match emitted `warn!`, so a diagnostic that aborts the run two statements later was
  indistinguishable in the log from one that changes nothing; field-classification errors were
  likewise logged as warnings immediately before bailing. Errors now log at error level.

- **The "requires FFI" warning no longer fires on a deliberate single-language regen.** It tested
  the `--lang`-filtered language list for an FFI entry, so `alef generate --lang csharp` warned
  that FFI was missing even when the FFI crate was configured, generated and committed. The
  condition it describes is a property of the crate's configured languages, not of one
  invocation's scope, and is now checked against those.

- **`alef verify` no longer reports a file as frozen once alef has durable proof it owns it.**
  The write guards in `write_files_report`/`write_scaffold_files_report` treat a marker-less file
  as owned when either it carries the provenance marker or — for formats with no comment syntax
  at all — the committed `.alef-ownership.toml` record says so. `alef verify`'s frozen-file report
  only ever checked the marker, so a file `alef adopt` had just recorded, or one a
  delete-and-regenerate had just rewritten and recorded, kept being reported "frozen" forever, even
  though the write guard would happily accept it. Both write guards and the report now share one
  `is_owned_by_ownership_record` predicate instead of three independently drifting copies.

- **`.clang-format` can now carry a provenance marker.** It is YAML underneath (`#` line comments),
  scaffolded `generated_header: true` for every FFI target, but was missing from
  `marker_header_syntax`'s file-name table — an oversight, not a deliberate exclusion like
  `DESCRIPTION`'s (which stays off the table on purpose; see that entry's doc). A pre-existing,
  unmarked copy previously reported frozen with no remedy to paste in; it now gets the real `#`
  header, the same way `Makefile`/`go.mod`/`Rakefile` already do.

- **`[e2e.call(s).*.overrides.<lang>] result_type` is now validated against the core IR,
  mirroring the `class` validation added in 0.62.2.** `result_type` names the struct/enum
  type a call's result binds to, and — for the `c` generator specifically — the value is
  baked verbatim into accessor/free symbols. Nothing checked it against the IR before it
  reached the emitter, so a typo surfaced late: either as uncompilable generated C, or as a
  wall of per-call "call did not resolve to a core IR function..." warnings once the
  misconfigured call's return type couldn't be derived any other way, both naming
  `result_type` as the fix. Generation now fails fast at config-validation time instead,
  with a did-you-mean suggestion against the type/enum names the crate actually declares. A
  `result_type` set to a primitive/pointer C spelling (`char*`, `int32_t`, ...) — a
  different misuse, where `raw_c_result_type` or `result_is_bytes`/`result_is_simple` was
  the field that belonged there — is now reported as its own warning rather than being
  silently accepted or conflated with an unknown-type error.

### Changed

- **Demoted `tracing::warn!` sites that fire on correct, working configurations to `info!` or
  `debug!`.** A triage of the 229 `warn!` call sites in `alef` found a set that were not
  reporting problems: an `[e2e]` block being detected (advice, not a defect), the CLI running
  newer or older than a project's pinned `alef_version` (expected after every release until a
  consumer bumps the pin), suppressed validation diagnostics re-printed despite
  `suppress_validation_codes` (now `debug!`, since re-warning defeats the consumer's own
  setting), a `precondition` skip (the consumer's own declared skip switch), an optional command
  failing or missing (the command is declared optional), the eleven "repaired pre-existing
  `<file>`" self-heal announcements (all fire only after the repair already succeeded), and the
  Swift artifactbundle build/checksum steps on hosts without Xcode. These no longer drown the
  warnings that matter in `generate`/`adopt`/`verify`/`diff` output.

### Removed

- **Removed three provably-unreachable code paths.** `check_signature_breakage`'s "no
  consumer-file scan is wired up" warning could never fire: every backend except Zig defaults
  `public_function_signatures` to empty, so the baseline comparison always short-circuits before
  reaching it for those languages, and Zig always has a non-empty `scan_extensions_for` entry, so
  its changes always take the attributed-caller branch instead. `ValidationReport::warnings()` was
  always empty because every diagnostic pushed into a `ValidationReport` is built with
  `ValidationDiagnostic::error`; the pipeline's own warning diagnostics travel through a separate
  `language_diagnostics` vec. Removed the dead `warnings()` method and its two always-empty
  iteration sites. The C# backend's `callback_specs_from_trait` (and its private
  `snake_to_lower_camel` helper and `CallbackSpec`/`ExtraParam` types) was only ever called from
  its own `#[cfg(test)]` module; removed the function, its helper types, and the test that only
  exercised it.

## [0.62.2] - 2026-08-19

### Fixed

- **C# no longer reports files as unemitted that the same run emitted.** The visitor-support check
  tested only whether a path existed on disk, and ran before the type and enum emitters had pushed
  anything. In the branch where visitor callbacks are off — which includes a consumer having no
  `[ffi]` section at all, since an absent section is indistinguishable from an explicit `false` —
  the candidates are `{context_type}.cs` and `{result_type}.cs` taken from `[[trait_bridges]]`, and
  those emitters go on to write exactly those files. Every `generate`, `adopt`, `verify` and `diff`
  on such a repo therefore warned about files it had just written. The check now runs after
  emission and excludes anything the run is actually writing.

- **Snippet validation no longer serializes languages that have nothing to serialize.** The
  per-snippet fallback runs inside a rayon pool, but every snippet took its session's mutex and
  held it across the whole toolchain subprocess, making the pass strictly serial per session. The
  mutex was introduced alongside the change that moved TypeScript, C# and Java onto a shared
  fingerprint-keyed workspace with fixed filenames (`snippet.ts`, `Program.cs`, `<Class>.java`),
  where concurrent snippets really would overwrite each other's sources mid-compile — but it was
  applied to every language, including the majority that write only into a per-call scratch
  directory. Validators now declare their own need via `requires_session_exclusivity`, and only
  those four (plus Kotlin, whose Gradle init script shares the workspace) are serialized. On a
  consumer repo this was 521 zig snippets of ~6s each running for half an hour.

- **`Starting per-snippet validation` no longer announces work that never happens.** The count came
  from the batch pass leaving an entry unclaimed, which conflates "not batched" with "will be
  validated". `validate_one` short-circuits on a cache hit, a `skip` annotation, a side-effect
  rejection, a missing validator and an unavailable toolchain. Because snippet validation runs once
  per crate with `changed_only`, later passes are almost entirely cache hits, so fully-cached
  languages reported `snippet_count=521` while doing nothing at all — making a run in which four
  languages fell back read as thirteen. The event is now emitted from the point where a toolchain
  is actually invoked, and the summary reports `resolved_without_toolchain` alongside it.

- **Per-snippet `duration_ms` no longer includes time queued behind other snippets.** The elapsed
  timer started before the session lock was acquired, so recorded durations were dominated by wait
  time — zig snippets doing ~5.9s of real work were recorded at a 58s median, which disguised the
  serialization above as per-invocation cost.

- **Generated C snippets no longer call a `_from_json` constructor the FFI never exports.** For an
  argument like `Vec<String>` the element type resolves to the std type `String`, and the e2e C
  generator built a typed handle from it — emitting `<prefix>_string_from_json("[]")` and a
  matching `<prefix>_string_free(...)`. The FFI crate exports `_from_json` / `_free` only for types
  the crate itself defines (and, for enums, only when one is used as a pointer parameter), so
  nothing declares those symbols and every snippet taking such an argument failed to compile with
  "call to undeclared function". The C ABI takes the argument as a plain `const char *` JSON string
  anyway, so std-typed arguments now skip the handle and are spliced in as a literal. Crate-defined
  types, including enums, are unaffected.

- **Bumping the `[workspace] alef_version` pin in `alef.toml` no longer rehashes every generated
  file.** `compute_inputs_hash` folded the entire normalized `alef.toml` into the embedded
  `alef:hash:` line, including the `alef_version` pin — so the standard consumer upgrade workflow
  (bump the pin) invalidated every file's hash with zero emitted-content change. The pin only
  feeds a version-mismatch warning (`cli::version_pin::check_alef_toml_version`); nothing in
  codegen branches on it. `alef_version` is now stripped from the canonical TOML before hashing;
  every other `[workspace]`/`[[crates]]` key is still a real input.

- **`[crates.e2e.call(s).*.overrides.<lang>] class` is now validated against the classes the
  target backend actually emits.** For java, kotlin, kotlin_android, php, ruby, and dart — the
  languages whose e2e generators read this field — a typo or a stale rename used to be trusted
  blindly by the emitter, silently producing hundreds of e2e tests and snippets that call methods
  on a class that does not exist, surfacing only as a wall of compile errors in generated code far
  downstream. `alef e2e` (and `alef build`) now checks every `class` override against the crate's
  facade class, every struct/enum wrapper, and every active trait bridge for that language, and
  fails generation with the offending config key, the bad value, and the closest valid
  candidate(s) by edit distance. The check is skipped when the caller supplies no IR (some
  legitimate generation paths do), matching the same rule the field-classification validator uses.

### Removed

- **Dropped the wall-clock companion to the subprocess-backoff test.** It timed 20 trivial commands
  and asserted the amortised cost stayed below the fixed interval the backoff removed, but bare
  process-spawn overhead on a loaded machine exceeds that bound, so it failed on load rather than
  on regression at two successive thresholds. The sibling test asserts the poll schedule directly
  and covers the same property without depending on machine load.

## [0.62.1] - 2026-08-19

### Fixed

- **`alef validate versions --json` no longer fails a release on a check that only the release can
  satisfy.** A test app's lockfile pins the crate at the version being published and resolves it
  from the registry, so cargo cannot refresh that entry until the version is live — which cannot
  happen while this gate blocks the publish job. alef already recognised the situation (the check
  is tagged `UNPUBLISHED`, and the human summary reports it as "unresolvable until the pending
  release is published"), but the JSON `ok` field still counted it as a failure, and
  `xberg-io/actions/validate-versions` fails on `ok != true`. The gate was therefore unsatisfiable
  by construction for any repo with a registry-depending test app, and it blocked the crates.io leg
  of a real release. `blocked_on_publish` checks are now excluded from `ok` while still being
  reported; a genuine mismatch sitting beside one still fails, and an empty check set is still not
  a pass.


## [0.62.0] - 2026-08-19

### Fixed

- **e2e suites no longer contain error assertions that can never pass.** A fixture's declared
  `error` value is either a message substring or an error variant name, and every backend rendered
  the same message-or-type-name disjunction for both. That serves the first convention and is
  structurally unsatisfiable for the second: the message is lowercase `#[error(...)]` prose that
  never contains the PascalCase identifier, and the "type name" side is a generic exception class
  the binding never differentiates per variant. Measured across two consumer repos this was the
  single largest class of e2e failure alef generates — Go 47/162, Java 47/162, PHP 47/162, Dart
  47/127, C# 45/162, Ruby 45/47, C 44/162, Zig 45. `declared_error_variant::classify` is now the
  one place that decides substantiability: Go, Java and Zig can still assert a variant that
  carries an `error_code`, and the backends that cannot emit a registered skip instead. Nothing is
  matched fuzzily to force the type-name side to work, and unaudited backends keep their existing
  behaviour.

- **The subprocess-polling regression test no longer fails under load.** It asserted that 20
  trivial commands finish in under *half* the 1s the old unconditional 50ms-per-subprocess sleep
  cost. That sleep ran before the first `try_wait` regardless of command speed, so any amortised
  cost below 50ms/command already proves it is gone on any machine at any load; the extra halving
  proved nothing and instead measured process-spawn overhead, which legitimately reaches 25ms+ per
  command on a loaded machine and failed the suite at 509ms and 527ms against a 500ms bound.

- **R bindings no longer drop every feature-gated function outright.** `extendr_module!` rejects a
  `#[cfg(...)]` on its entries ("expected mod, fn or impl"), so R cannot gate a registration the
  way Magnus gates its `define_module_function` call. The workaround was to exclude any genuinely
  cfg-gated function from both the registration block and the R wrapper surface — unconditionally,
  whether or not the feature was actually enabled — so a crate with a cfg-gated function could
  never expose it through R even in the default build, silently. The predicate is now resolved
  before generation, exactly as the field policy beside it already did: an enabled function
  reaches R with its gate discharged, and a disabled one is removed outright so no
  `extendr_module!` entry or wrapper can name a symbol the crate never compiled.

- **Ruby bindings no longer fail to build when a function is feature-gated.** `prepend_cfg` put
  `#[cfg(feature = "X")]` on the generated `fn`, but the registration loop in `gen_module_init`
  never read `func.cfg` and emitted the `module.define_module_function(..., function!(...))` line
  unconditionally. With the feature off the definition compiles out while the registration still
  names it, so the binding crate fails with `E0425` — a broken build, not a missing Ruby method.
  `#[magnus::init]`'s body is a flat statement list, so the attribute on the registration
  statement is the only place the gate can go; the method loop directly above already did this
  via `method.cfg`.

- **`#[serde(untagged)]` data enums no longer lose their payload in WASM bindings.** `gen_enum`
  special-cased internally-tagged data enums only. An untagged one has no serde tag, so it fell
  through to the fieldless path and was emitted as a bare discriminant enum with every variant's
  payload replaced by `Default::default()`; the containing struct's setter then took that
  fieldless enum, so no JS caller could supply the string or array the variant actually carries.
  These now bridge to `JsValue` through `serde_wasm_bindgen` — the mechanism this backend already
  used for internally-tagged data-enum fields — so wasm-bindgen emits `any` for the property and
  the `.d.ts` and the runtime setter agree by construction. NAPI already gated on the same
  `EnumDef::serde_untagged` flag. Affects `EmbeddingInput`, `ModerationInput`, `StopSequence` and
  `ToolChoice` in `liter-llm`.

- **The NAPI `.d.ts` can no longer contradict itself about a type's name.** `dts_type` and its
  siblings each took a `prefix: &str` that all eleven-plus call sites had to remember to pass as
  `""`; passing the real prefix at any one of them emits `Array<JsMessage>` against a type
  declared as `Message`. NAPI-RS wraps `Foo` as `JsFoo` in Rust and maps it back via
  `#[napi(js_name = "Foo")]`, so the `.d.ts` — which describes the JS boundary, not the Rust
  struct — must use the identity name everywhere. `codegen::naming::node_type_name` is now the one
  place that decides this and the parameter is gone, so the declaration site and the reference
  site cannot drift.

- **Two Dart e2e regression tests no longer assert a naming convention alef deliberately
  dropped.** `b5808da3c` stopped emitting leading-underscore Dart locals — Dart privacy is
  library-scoped, so the prefix carried no meaning and only tripped
  `no_leading_underscores_for_local_identifiers`, failing 188 of 207 published snippets under the
  `dart analyze --fatal-infos` that alef itself runs. That commit updated
  `e2e_dart_client_factory.rs` but missed `e2e_generic_call_recipe.rs` and
  `e2e_unified_extract_input_args.rs`, which kept expecting `_settings`/`_input`/`_config`. The
  generator was right and the expectations were stale; both now assert the lint-clean names. The
  same test file also carried a real consumer project name through its fixture config and all
  three language assertions, which `project-agnostic-codegen` forbids — renamed to a neutral
  fixture identity.

- **Generated Go e2e files no longer carry an unused `strings` (or `os`) import.** Go rejects an
  unused import outright, so `tree-sitter-language-pack`, `liter-llm` (6 files) and `crawlberg`
  (3 files) all had e2e suites that could not compile. Both flags are fixture-level heuristics —
  "some assertion is of a kind that might want this package" — and deliberately a superset, since
  an assertion can be skipped, degraded to a stub, or rendered without ever naming the package.
  They were OR-ed with the rendered body rather than narrowed by it, so the heuristic alone forced
  the import; they now match how `needs_fmt` and `needs_pkg` on the adjacent lines already
  authorise themselves against the body they actually produced.

- **Whitespace-control tags no longer eat the indentation of generated e2e code.** The e2e
  template environment already sets `trim_blocks` and `lstrip_blocks`, so a plain `{% %}` tag
  strips exactly the one newline that separates it from its content. An explicit `-` on top of
  that strips *all* whitespace to the next non-whitespace character, which deletes the emitted
  statement's own indentation and, on a `{% for %}`/`{% endfor %}` pair, the newline between
  iterations. Two of the results were not cosmetic: `python/app_harness.py.jinja` glued an
  assignment onto the preceding comment line, so `_config` was never defined and the next
  statement raised `NameError`; and `r/test_case.jinja` concatenated setup lines into
  unparseable R (`x <- 1res <- foo(1, 2)`). `csharp/http_test_open.jinja` and
  `java/http_test_open.jinja` also turned out to be a second path that collapsed `[Fact]`/`@Test`
  onto the method signature, distinct from the `test_method.jinja` path fixed in 0.61.1. Redundant
  `-` modifiers are removed across the csharp, java, php, ruby, typescript, swift, go, r, python
  and elixir templates, with layout tests pinning the exact emitted indentation for the C# and
  Java assertion templates.

- **Generated e2e assertions now read a field's optionality from the IR instead of a
  hand-maintained config table.** `FieldResolver`'s `optional_fields` was populated only from
  the consumer's `[crates.e2e] fields_optional` list in `alef.toml`, never from `FieldDef.optional`
  — which extraction already sets correctly, and which every language backend already consults to
  wrap the field in that language's optional type. A consumer that declared no `fields_optional`
  therefore got assertions that dereference an `Option` directly: `assert!(result.data, "expected
  true")` against `pub data: Option<DataNode>` does not compile, and the equivalent in twelve other
  backends either fails to compile or is silently false at runtime. `FieldResolver::ir_field_sets`
  now also derives an optional-field set and `with_ir_fields` merges it into the config-declared
  one, so `fields_optional` remains an override for what the IR cannot see rather than the only
  source of truth. Derivation is deliberately unanimous — a bare field name counts as optional only
  when *every* type declaring it marks it `Option<T>` — because a false positive here emits code
  that does not compile, whereas a false negative merely reproduces the previous behaviour.
  Alongside it, `is_true`/`is_false` now mean "present"/"absent" for an optional field in rust, go,
  java, python, kotlin, kotlin_android, dart, swift, php, ruby, elixir, typescript and zig, matching
  the convention the Rust backend already used; csharp and c were already correct. The shared
  doc-snippet path (`e2e::codegen::presentation::resolve`) is wired to the same IR data, so a
  snippet showing an optional field renders the same unwrap an assertion on it would.

### Added

- **`alef build` warns when a binding crate never declared a feature its generated source
  references.** `scaffold` computes each binding crate's `[features]` table once and `alef build`
  never revisits it, so a cfg-gated symbol added to the core crate after the last scaffold run
  resolves against a feature the binding crate does not declare — false, unconditionally. For Ruby
  that now surfaces as a build error; for Elixir nothing surfaces at all, because `#[rustler::nif]`
  gates a definition and its registration atomically, so the NIF is simply absent while the
  generated facade still advertises it. `warn_on_undeclared_binding_cfg_features` reads the
  scaffolded `Cargo.toml` back off disk and names the missing features. A warning rather than a
  hard error or an automatic rewrite: `Cargo.toml` is scaffold-owned and written once by design.

## [0.61.1] - 2026-08-19

### Fixed

- **`cargo publish` runs again.** The publish workflow gated every downstream job on a
  `validate-versions` step that ran `alef validate versions` against alef itself. That check
  exists to confirm a *consumer's* package manifests agree with the crate version; alef is the
  generator — a single Rust crate with no target-language packages — so it had nothing to
  validate, and crate resolution rejected the config outright with "crate `alef` has no target
  languages". Because `publish-crates` required `needs.validate-versions.result == 'success'`,
  the failed gate skipped the actual publish while the release itself still looked created. That
  is why 0.61.0 never reached crates.io, and why the last version published there was 0.60.1.
  The job and the fictional `[[crates]]` block in `alef.toml` that had been added to satisfy it
  (b00e72d60) are both removed.

- **The xUnit attribute on a generated C# e2e test no longer collapses onto the method
  signature.** `test_method.jinja` picks between `[Fact]` and `[Fact(Skip = "...")]` in a
  conditional; written with whitespace-trimming delimiters, that block also ate the newline after
  the attribute, emitting `[Fact]    public void Test_X()`. The result still compiled, so nothing
  failed -- every generated C# e2e suite simply carried the mangled line. Regression from 0.61.0,
  now pinned by tests that assert the attribute occupies its own line in both branches.
- **A struct field defaulted only through `<FieldType>::default()` now resolves to a concrete
  value instead of forcing every generated language binding into an unconstructible `required`
  member.** The extractor folded `SomeEnum::default()` (and `Default::default()`) to
  `DefaultValue::Empty` regardless of the field's own type, which is correct for a primitive,
  string, or collection field but ambiguous for an enum-typed one -- `Empty` names "the type's own
  zero" without saying which variant that is. A new postprocess pass,
  `extract::extractor::postprocess::resolve_enum_field_defaults`, narrows `Empty` on an
  enum-typed field to `DefaultValue::EnumVariant` when the enum's default variant is known. To
  make it known in the hand-written case, `impl Default for SomeEnum { fn default() -> Self {
  Self::Variant } }` is now read directly and its variant marked `EnumVariant::is_default`;
  previously only `#[derive(Default)]`'s `#[default]` attribute set that flag, so every consumer
  of it (the Go, Rustler, Dart, WASM, Kotlin, Magnus and PHP backends, and the generated Rust
  mirror enum's `#[default]` marker) silently fell back to the first declared variant or to no
  default at all. Only a bare unit variant is narrowed: a tuple or struct variant needs
  `TupleVariant`/`StructVariant` and a payload this pass cannot read, and emitting a bare variant
  name for one would fabricate a value that does not compile. An enum whose default variant stays
  unknown is left `Empty`, preserving every backend's existing honest fallback.
- **C# no longer emits a `required` member for a struct-typed field whose nested record is itself
  fully default-constructible.** A field defaulted only by a container-level `impl Default` (no
  per-field `#[serde(default)]`, so the sole signal is `Empty`) now reuses the existing
  `record_is_default_constructible` walk rather than falling through to `required`. A nested
  record that carries a `required` member of its own still correctly keeps the outer field
  `required`. Together with the fix above this resolves a real regression: a record with several
  enum fields and a nested-struct field, each defaulted only via `T::default()`, emitted a
  `required` member for every one of them, making a bare `new Record()` -- exactly what alef's own
  snippet generator emits for a type with no constructor arguments -- fail to compile with
  `CS9035` on every generated snippet that touched the type.
- **Kotlin/Android snippet validation resolves a real Gradle classpath instead of guessing a
  directory layout.** `KotlinValidator::class_path` probed exactly three fixed directories
  (`build/classes/kotlin/main`, `build/classes/java/main`,
  `build/intermediates/javac/debug/classes`) plus `build/libs/*.jar`, falling back to the project
  root when none existed. AGP's actual compiled-output path is variant- and version-dependent (AGP
  9.x lands classes at `build/intermediates/built_in_kotlinc/debug/compileDebugKotlin/classes`,
  matching none of the three probes), and directory probing can never see a project's *dependency*
  classpath at all -- so every snippet touching a dependency-typed symbol (kotlinx-coroutines,
  Jackson DTOs, ...) failed with `unresolved reference`, and the fallback-to-root path made every
  other snippet fail too. A Gradle manifest (`build.gradle.kts` / `build.gradle`) with a `gradlew`
  wrapper is now resolved by asking Gradle itself, via a `--init-script` that matches every
  `compile*Kotlin` task by name and prints its destination directory and resolved classpath --
  no consumer build-file change required. The resolution is cached per manifest for the process
  lifetime, since batch validation calls it once per batch and a Gradle invocation costs whole
  seconds even warm. A Gradle invocation that fails still falls back to the original directory
  probing rather than failing the session outright.

- **Zig snippets stop rebinding a discarded value that is not a call.** The generator rewrites a
  statement-opening `_ = <call>(...)` into `const result = ...` so the snippet can show its result,
  but the rule matched any `_ =` discard. Every generated visitor callback opens by discarding its
  unused typed parameters (`_ = _ctx;`, `_ = _user_data;`, `_ = out_custom;`), and those lines
  precede any real call in a visitor body, so the first one became `const result = _ctx;` -- a bound
  value nothing reads, which Zig rejects outright as an unused local constant. A call discard is
  syntactically distinct from a bare-identifier discard, carrying a parenthesised argument list, and
  the rule now requires one.

- **Snippet generation honours a language's visitor exclusion, as e2e test generation already did.**
  A per-language `exclude_functions = ["visitor"]` drops the fixture engine's trait-bridge entry point,
  and `e2e::codegen::kotlin_android::project` already fell back to an excluded-bindings placeholder for
  it. The snippet generator applied no such rule, because `exclude_functions` normally names a real Rust
  function while a visitor fixture's *call* resolves to an ordinary one (`convert`) and the visitor
  itself attaches through an options field that has no IR function name of its own. The two generators
  therefore disagreed about the same config, and every visitor fixture was rendered as a real snippet
  against an API the binding never exposed -- 46 of them for one consumer, each importing a visitor
  interface, a node-context type and a result enum that are absent from the generated package. The token
  is now a single named constant both generators read, so they cannot drift on which fixtures an
  exclusion covers.

- **One binary file no longer ends an `alef adopt` run.** Candidate collection read every match with
  `read_to_string`, so a single non-text match aborted the whole target: `alef adopt 'packages/**'`
  on a repo with a `gradle-wrapper.jar` failed with `stream did not contain valid UTF-8` before one
  of the hundreds of adoptable text files under the same glob was stamped, and no narrower target was
  suggested. Binary matches are now collected separately and reported under a `NOT ADOPTED -- not text`
  heading. They are still never adopted: a drifted path is only ever adopted after its diff is printed,
  and a binary artifact has neither a diff to review nor a syntax that could hold a provenance marker.

- **C# `[DllImport]` parameters now derive their width from the same fact as the emitted C signature.**
  `marshalling::pinvoke_param_type` mapped every `TypeRef::Named` to `ulong`, but the C FFI backend
  narrows a `Named` parameter whose type is `Copy` to `i32` — cbindgen renders that `int32_t`. A `Copy`
  enum parameter was therefore declared eight bytes wide against a four-byte signed argument, and the
  wrapper's own `(int)` cast (`named_param_enum_required.jinja`) could not even be passed to it, so the
  generated package failed to compile with `CS1503: cannot convert from 'int' to 'ulong'`. Casting the
  argument to `ulong` would have silenced the compiler while cementing the ABI violation; instead the
  scalar/handle split is now constructed once, in `backends::ffi::type_map::scalar_c_abi_named_types`,
  and read by the C FFI backend, both P/Invoke emitters and the service-API emitter. The C# service-API
  path additionally stopped keying that decision off enum-ness, which disagreed with the C header for a
  non-`Copy` enum (boxed as a handle) and for a `Copy` struct.
- **C# `bool` parameters cross the C ABI as `int32`, not a one-byte managed `bool`.** The free-function
  and method P/Invoke emitters declared `[MarshalAs(UnmanagedType.U1)] bool`, which marshals one byte,
  while the C FFI crate declares `i32` and cbindgen emits `int32_t`. The callee reads four bytes, so the
  upper three were whatever the calling convention left there — reachable in practice for any argument
  passed on the stack rather than in a register. Every other boundary in the same backend already
  agreed on `int` (trait-bridge delegates, the service-API map, and the `bool` *return* mapping), so the
  P/Invoke declaration was the outlier; the wrapper now passes `(value ? 1 : 0)` to match.

- **`alef snippets check --lang` accepts the session names a user actually has.** The filter resolved
  its values as fence tags only, so every session target whose name differs from its fence tag —
  `kotlin_android`, `node`, `wasm`, `c_ffi` — was rejected as unrecognised. Those names are the only
  ones a consumer has for those sessions, because they are the keys of the `[workspace.docs.snippets.sessions]`
  table they were just reading, and the rejection meant the one language they most needed to narrow to
  could not be selected at all. Session targets and their `-`/`_` spellings now resolve alongside fence
  tags, aliases of one language collapse to a single entry, and the error names the values it could not
  resolve instead of listing the ones it could.

- **Zig packages no longer search an FFI include directory guessed from the crate name.** `scaffold_zig`
  started deriving the `-Dffi_include_path` default from `[crates.output] ffi`, but `packages/zig/build.zig`
  is a create-once seed, so every repo scaffolded before that kept searching `crates/<crate-name>-ffi/include`
  forever. Where that is not the real FFI crate directory the binding's `@cInclude` never resolves and
  `zig build` — along with every generated Zig documentation snippet — fails with `C import failed` /
  `'<header>.h' not found`. A migration now repairs the default in place, and only when it still matches
  the crate-name-derived shape, so a consumer who repointed the option keeps their value.
- **Generated `build.zig` resolves its FFI library and include defaults against its own build root.** Both
  are attached with `.{ .cwd_relative = ... }`, which zig resolves against the invoking process's working
  directory, so the raw relative defaults only found anything when zig ran from inside `packages/zig`:
  `zig build --build-file packages/zig/build.zig` from the repo root failed to open `../../target/release`,
  and consuming the package as a `.path` dependency — which is exactly how the Zig snippet validator builds
  it — failed to find the header. The Zig snippet validator reads the rebased binding back correctly, and
  now resolves manifest-declared include paths against the manifest's own directory rather than the
  session's working directory.
- **`alef adopt` no longer lets one unusable target cancel every remaining one.** `run` bails whenever a
  target resolves to nothing adoptable — no match, or (far more common on a repo-wide sweep) only
  create-once seeds — and that error propagated straight out of the per-target loop. A single `config.m4`
  early in a sorted list of 54 refused paths therefore ended the command before one file was stamped,
  reporting only that path. Each target is now reported independently and the run fails at the end iff
  any did.

### Added

- **`alef snippets check --lang <tag>`** validates only the named languages. Diagnosing one backend's
  snippets previously meant paying for all of them: a full consumer tree is thousands of snippets across
  sixteen toolchains. The audit and gap passes still see the whole corpus, because an unreferenced snippet
  or a missing language variant cannot be judged from a subset.

### Changed

- **A batched invocation's timeout scales with the number of snippets it covers.** `timeout_secs` is a
  per-invocation budget, and while only Rust batched a "batch" was a handful of snippets. Now that one
  `tsc` or `dotnet build` covers a language's several hundred, a flat grant would kill the group as a
  toolchain timeout long before the compiler finished — a failure mode batching itself introduced. The
  grant stays far below the serial path's total, since paying one startup instead of N is the whole point.
- **Snippet batch groups and session preparation now run concurrently.** Batch groups holding different
  session locks were dispatched from a plain sequential loop, so the whole pass cost the sum of every
  language rather than the slowest one; group results are now merged back at their snippet positions
  after concurrent dispatch. Session preparation was likewise serial — sixteen languages meant sixteen
  `pnpm build` / `mvn package` / `cargo build --release` hooks back to back before a single snippet was
  validated. Preparation now parallelizes across distinct working directories only, because two sessions
  sharing one must not run their `before` hooks at the same time, and the scratch purge still runs
  strictly between the resolve and activate phases (it needs the complete set of live fingerprints).
- **The Rust snippet batch reuses a persistent, fingerprint-keyed `CARGO_TARGET_DIR`.** It allocated a
  fresh scratch directory per run with no target directory set, so `cargo check` recompiled the path
  dependency and its entire transitive tree from cold on every single run.
- **`session_fingerprint` no longer hashes build output.** Its exclusion list covered six directories and
  missed `dist`, `bin`, `obj`, `_build`, `vendor`, `Pods`, `.gradle`, `.next`, `.dart_tool`, `.zig-cache`
  and `__pycache__`, so a repo with built artifacts hashed hundreds of megabytes per session per run.
  Hashing is now parallel over a path-sorted file list, which keeps the digest stable across runs — a
  fingerprint that varies invalidates the whole cache silently.
- **Subprocess waiting backs off from 1ms instead of polling on a fixed 50ms sleep**, removing ~25ms of
  pure sleep per subprocess. Timeout semantics are unchanged: the final sleep is clamped to the remaining
  budget.
- **The docs snippet pass reads the cache it writes.** It built its `RunnerConfig` from a default with
  `changed_only: false` while setting `cache_dir`, so every run wrote an entry per snippet and read none
  back — a guaranteed 100% miss.
- **C, Dart, Elixir, PHP, R, Ruby and Zig snippets validate in one invocation per language**, closing
  the batching sweep: `cc -fsyntax-only` and `dart analyze` take every file at once, and Elixir, PHP,
  R and Ruby each run one interpreter over a checker script that reports per file. Two toolchain
  findings drove the shape: `ruby -c a.rb b.rb` checks only the FIRST file and hands the rest to the
  script as `ARGV`, so a broken second file passed silently; and `zig ast-check` refuses a second
  path outright, so the Zig batch goes through `zig fmt --ast-check`. Levels that link an executable
  decline and fall back, since each `main` needs its own artifact.
- **Swift deliberately does NOT batch.** `swiftc` compiles one module per invocation and a module
  permits top-level code in exactly one file, so `swiftc -parse a.swift b.swift` fails the other
  snippets with "expressions are not allowed at the top level" before judging their own code. There is
  no way to scope a snippet's top-level statements into its own namespace, so batching would fail
  snippets for a reason the per-snippet path never had.
- **Java, Kotlin and C# snippets validate in one compiler invocation per language instead of one per
  snippet.** Java and Kotlin collapse a JVM startup per snippet into one; C# collapses a `dotnet build`
  per snippet into one. Measured on 20 snippets: `javac` 5.44s to 0.23s, `kotlinc` ~76s to 4.86s,
  `dotnet build` 15.7s to 1.36s against an already-warm project directory. Each snippet is given a unique
  synthetic package/namespace so two snippets declaring the same top-level name cannot fail each other —
  a batch is declined outright when two snippets declare the *same* explicit package, since that
  collision is real and only the compiler can tell it apart from the synthetic case. One consequence is
  deliberate: entry-point-ness is a project-level property in C#, so a batched project builds as a
  library and a snippet that used to fail `CS5001` (no static `Main`) now passes.
- **TypeScript, Python and Go snippets validate in one compiler invocation per language instead of one per
  snippet.** Only Rust batched before; every other language paid a full `tsc` / interpreter / `go build`
  startup per snippet, and all of a language's snippets were serialised behind its session lock, so the
  cost was fully additive. On a consumer tree with ~283 snippets per language this is 283 processes to 1
  at every non-`Run` level. `Run` still validates per snippet by design — each one's stdout, exit status
  and side effects belong to it alone. Batch diagnostics are attributed back per file, and a compiler
  failure that no file owns fails every snippet in the batch with the real output rather than passing them.
- **Java snippets call a class in the imported package by its simple name.** The snippet emits
  `import <package>.*;` and then spelled the package again at the call site, rendering
  `io.example.pkg.Facade.convert(...)` under an import that made `Facade` alone sufficient. Only the
  exact configured package is stripped; a nested or foreign class stays qualified, since no import
  covers it.
- **Python snippets omit a trailing `None` for an absent optional argument.** `convert(html, None)` now
  renders as `convert(html)`, matching the binding's own `options=None` signature. A placeholder in the
  MIDDLE of the argument list is still emitted — these calls are positional, so dropping one there would
  slide the following argument into the wrong slot.

- **`alef adopt` and `alef verify` render the managed surface in parallel.** Every stage reads the same
  IR and config and returns owned files, so rendering them concurrently is safe; absorption stays strictly
  sequential because `absorb_stage` is last-wins on a path collision and the fold order is what decides
  which stage owns a contested path. The two e2e stages alone emit several thousand files each on a full
  consumer tree, which was most of what made a single-path `alef adopt` cost half a minute.

- **An uppercase `.R` script now receives the generated header.** `.R` is the conventional extension for an R
  script and alef emits `install.R`, `run_tests.R` and every `packages/r/R/*.R` with `generated_header: true`, but
  the emit predicate matched a lowercase `"r"` only — so every one of them was written unstamped and then frozen
  by the write guard for want of a marker nothing had been emitting. Extension matching is now case-folded on the
  emit side only; the ownership predicate is deliberately unchanged, because reclassifying `.R` from unmarkable to
  markable would retroactively freeze every already-committed `.R` that proves ownership through the record.

- **A stripped `# Arguments` bullet no longer leaks its own continuation lines into the reference
  page.** The two arms that recognise a wrapped bullet tested the TRIMMED line for leading
  whitespace, which cannot match by construction, so the skip ended at the first wrapped line and
  published the bullet's tail — mid-sentence, with no heading above it — into every generated
  reference page.
- **Doc-comment sections nest under the item that owns them.** Function, error, enum and streaming
  pages shifted rustdoc headings by a fixed number of levels, which assumes the doc comment starts at
  `#`. A `# Observability` section under a `####` item surfaced as `###`, reading as a sibling of the
  page's `### Functions` section and taking a bogus table-of-contents entry with it. Each now demotes
  so the first heading starts one level below its own parent.
- **A C# snippet's visitor class is emitted at file scope without the e2e test class's nesting
  indent**, and the batch validator finds the statement/declaration boundary by brace depth rather
  than by column. Either half alone left the class inside the wrapper method, where C# does not allow
  one: 54 of one consumer's 283 C# snippets failed on `CS1513: } expected`.
- **Zig snippets no longer rebind the allocator teardown.** The rewrite that names the discarded call
  result ran per line with no guard, so it also matched the teardown every snippet emits:
  `defer _ = gpa.deinit();` became `defer const result = gpa.deinit();`, which no Zig grammar accepts.
  It failed 54 of one consumer's 283 Zig snippets on `expected block or expression`. The rewrite now
  requires the discard to open the statement, and applies once per body.
- **The generated `build.zig` resolves its FFI search paths against its own build root.** Both were
  attached with `.{ .cwd_relative = ... }`, which resolves against the invoking process's working
  directory — so the package built correctly only when invoked from its own directory, and failed as a
  `.path` dependency or from the repo root.
- **A stale `-Dffi_include_path` default is repaired in place.** `build.zig` is create-once, so a repo
  scaffolded before the default was derived from `[crates.output] ffi` kept a path guessed from the
  crate name (`crates/<crate>-ffi/include`) that alef could never correct. The migration fires only when
  the on-disk value still matches that guessed shape and differs from what this run generates; a
  consumer who repointed the option keeps their value.
- **A Go visitor fixture attaches its visitor to the options value the call already binds.** The
  generator unconditionally introduced a second `opts` object, which was wrong twice over: the call then
  carried both bindings — `Convert(html, &options, opts)`, a hard "too many arguments" from the Go
  compiler, because the substitution helper only recognised a literal trailing `nil` and appended in
  every other case — and the fresh empty object silently discarded whatever options the fixture had
  configured.
- **A snippet result that comes back `Unavailable` is now reported per language, with the validator's
  message.** Only `Fail` and `Error` were tallied, but `Unavailable` fails the run under `strict`, and the
  `unresolved_dependency` reclassification turns a real validator failure — diagnostic and all — into one.
  That is how 566 snippets across two languages reached the final summary as "283 unresolved dependency"
  apiece without one line anywhere saying *which* dependency, while the message sat unread on every
  result. A language whose every result came back unvalidated also no longer logs like a clean pass.
- **Go snippets pass an absent options object by address when the binding takes a pointer.** Six of the seven
  `json_object` branches in the Go snippet argument builder consult `options_ptr`; the native-DTO branch that
  handles a fixture supplying *no* options never did. On any crate whose options parameter is `Option<T>` — a
  `*T` in the emitted Go signature — every optionless fixture, which is most of them, produced
  `Convert(html, options)` against `func Convert(html string, options *ConversionOptions)` and failed to compile.
- **Generated snippet bodies no longer open a blank line before the closing fence.** Generators hand the renderer
  a body already ending in a newline and the template emits its own, so every generated code fence carried a
  trailing blank line.

- **A subscripted `fields_optional` / `fields_array` entry is a claim about the element, not the container.**
  `validate_field_classifications` stripped the `[...]` suffix and ruled on the field it was attached to, so
  `metadata.document.open_graph[title]` — a key lookup on a `HashMap<String, String>`, optional in every host
  binding — was reported as "contradicts the core IR" and failed the whole run. The map is precisely the right
  home for an optional key lookup and the wrong home for an optional bare field; one predicate cannot judge both.
  Subscripted entries now resolve one level through the container: `Optional` clears anything subscriptable,
  `Array` requires the element itself to be indexable, and a subscript against a scalar is still an error whose
  message says so.

### Removed

- **`poly.toml` no longer schedules snippet validation as a pre-commit hook.** Snippet validation compiles every
  snippet against built language artifacts — minutes of work needing a toolchain per target language — and
  `alef all`'s docs stage already runs it against the tree it just generated. Running it again from a git hook
  made a one-line docs edit pay for a full multi-language compile. Regenerate to drop the
  `[hooks.pre-commit.commands.alef-snippets]` table.

## [0.61.0] - 2026-08-18

### Added

- **`MethodDef` carries its `#[cfg]`**: it was the only IR node without one, and extraction saw the attribute and
  discarded it. Methods now inherit their impl block's gate (AND-combined) and survive `with_cfg_filtered_deep`, so
  each backend filters against its own feature set — one surface is extracted once and handed to every backend in
  parallel, so dropping at IR level is impossible. `cbindgen_feature_defines` moved in lockstep: it is a second,
  independent feature collector, and a feature missing from `[defines]` makes cbindgen emit the declaration
  **unguarded**, which is worse than not gating. **Behavioural risk**: a language whose `features_for_language`
  omits a feature the cdylib was built with now loses those methods — the divergence `warn_on_ffi_feature_drift`
  announces. Native `#[cfg]` emission for pyo3/napi/magnus/rustler/dart, swift method filtering, and gleam are
  deliberately deferred: each needs the method and its runtime registration gated together.
- **`TypeDef::serde_container_default`**: `FieldDef::default` is populated only from per-field attributes, so a
  struct carrying `#[serde(default)]` at the type level was indistinguishable from one with no defaults at all.
- **`DefaultValue::Unresolved`**: `Empty` meant two opposite things — "the default is exactly the type-zero" and
  "the extractor could not read it". `has_default` cannot separate them (it is set for a manual `impl Default`
  too), so the distinction lives in the value. The extractor now follows `Self::new(<literals>)` delegation to
  recover real values, and refuses at validation time when it cannot, with `suppress_validation_codes` as the
  release valve. This is the shape that shipped `DetDbThresh = 0.0f` into generated C# beneath a doc comment
  reading "default: 0.3". Still conflated: a field initializer that cannot be read *inside* an otherwise-readable
  struct literal, which remains `Empty`.
- **`validate versions` distinguishes unresolvable-until-publish drift from stale manifests.** A `Cargo.lock` whose
  own manifest depends on a workspace crate *from the registry* at exactly the version being released cannot be
  refreshed at all until that release is published — cargo cannot resolve `x = "1.15.1"` while the index tops out
  at `1.15.0` — so the lockfile stays pinned to the last published version. That row used to be indistinguishable
  from a chore somebody forgot. It is still a mismatch and still fails `--exit-code`, but it now prints as
  `[UNPUBLISHED]`, its summary line reads `unresolvable until <name>@<version> is published`, and the JSON payload
  carries a `blocked_on_publish` field per check. A dependency taken by `path` is refreshable today and stays
  plain drift.

### Changed

- **`exclude_functions` is now actually enforced** — and for some consumers this removes symbols at upgrade with no
  other warning. The key has been honoured inconsistently: a downstream repo declared four exclusions in July and
  three subsequent regenerations on 0.60.x emitted the functions anyway. This release closes that gap across Go
  (`func` declarations), Java/Panama (methods, both `MethodHandle` downcalls, and the symbol-lookup and
  not-found-message entries) and Ruby (`.rbs` signatures). If your `exclude_functions` was silently a no-op, the
  named functions disappear from the generated surface on the next regeneration. That is the configured behaviour,
  but it arrives as a breaking change to the emitted API. **Why it hid for a month**: where a C ABI exposes only
  the async variant of a function, the async pair is the *only* observable test of the exclusion, so entries naming
  a non-existent sync symbol are unfalsifiable.
- **Generated binding crates now always carry a `[lints.clippy]` deny block** (`dbg_macro`, `print_stderr`,
  `print_stdout`), instead of emitting one only when a consumer declared `[crates.cargo_lints.clippy]`. Binding
  manifests are `generated_header: true` and therefore rewritten in full on every run, so an opt-in block that a
  consumer had hand-added into the `DO NOT EDIT` manifest was deleted on each regeneration — silently removing the
  logging enforcement the block existed to apply. A consumer-configured value for any of these keys still wins, so
  a crate with a real reason to relax one can. No alef template emits `println!`/`eprintln!`/`dbg!` outside
  `build.rs`, whose `cargo:` directives are natively exempt.

### Fixed

- **emitter, so a literal that had fallen behind wrote itself back over the consumer's own bump on every run — the
  reported shape was a repo on `base64 = "0.23"` being handed `base64 = "0.22"` and hand-reverting it after each
  regeneration. Before a rendered manifest is returned, each requirement is now compared against the one the
  committed manifest declares for the same crate and the higher lower-bound wins. **This makes emitted manifests a
  function of disk state, not of config alone** — deliberately, and it converges in one pass. It also means a
  consumer pinned *ahead* of what a generated shim compiles against now gets a compile error at the version they
  chose rather than a silent downgrade. The floor declines to rule rather than guess: a requirement with no lower
  bound (`*`, `<2`), an entry with no `version` key (`foo.workspace = true`, path-only deps), an unparseable
  requirement, or a manifest that is not valid TOML all leave the emitted value untouched.

- **scaffold**: the `[lints.clippy]` rationale alef stamps into every generated binding manifest now carries a
  `~keep` marker. It did not, so poly's uncomment pass — which runs *between* regenerations and strips any
  comment without a marker — deleted it, and the deletion landed in a commit that read as unrelated formatting.
  Where alef overwrites a consumer's own `~keep`-marked rationale above that block (it does; these manifests have
  no comment-preserving merge, unlike `poly.toml`), what replaces it is now at least as durable as what it
  displaced. This is the mirror image of 0.61.0's fix for alef *leaking* `~keep` into generated output, and does
  not collide with it: `strip_internal_doc_markers` runs only inside `normalize_rustdoc`, on doc comments harvested
  from a consumer's Rust source, never on scaffold-emitted TOML. The literal also no longer carries trailing
  whitespace on every line, which made the in-memory `GeneratedFile` content disagree with the whitespace-normalised
  bytes on disk.
- **renovate**: the `customManager` regex matched `pub const [A-Z_]+`, which excludes every constant whose name
  contains a digit. `PYO3` and `PYO3_ASYNC_RUNTIMES` were therefore never bump-proposed and had no way to be — and
  a const is indistinguishable, from outside, from one nobody has needed to bump. `base64` and `jni` are hoisted
  out of `scaffold::languages::jni` into `template_versions.rs` in the same change, which only means anything
  now that the regex can see them. `phpunit/phpunit` and `guzzlehttp/guzzle` remain unreachable on purpose: their
  rationale comments sit between the marker and the const, and their compound `||` constraints span several majors
  deliberately, so an auto-bump would collapse exactly what they exist to express.
- **records**: indent `.alef-toml-merge-provenance.toml` arrays like `.alef-ownership.toml`. The two records sit
  side by side in a consumer's repo root and pass through the same `poly fmt --check` gate, but the array indent was
  derived twice: the ownership record hand-rendered two spaces while the provenance record inherited four from
  `toml::to_string_pretty`, whose pretty serializer hard-codes `"    "` per element. Every generated tree therefore
  carried one standing "would reformat" file that no consumer could repair — hand-formatting is overwritten by the
  next `alef generate`, which the record's own header says. The provenance record is now rendered like the
  ownership one, both read the indent from a single constant, and a test compares the two writers' actual output so
  the next divergence fails whichever side moves. Regenerating rewrites the whole record once, whitespace only.
- **validate**: `alef validate versions` now discovers manifests through git, not through a disk walk, and reports
  which mismatches cannot be fixed in-tree. A consumer at `1.15.1` got five mismatches of which two were real. Two
  rows described `packages/ruby/tmp/ruby/stage/**` — gem-build staging, `tracked=no, ignored=yes` — whose tracked
  originals were correct. The third was worse: a *tracked* `Cargo.lock` reading `1.15.1` was reported as a
  mismatch while every other `1.15.1` row printed `ok`, because the staged copy of that crate's `Cargo.toml`
  declares the same package name at the previous version, and `cargo_manifest_versions` keys its map by name alone
  — glob order puts the staged copy last, so it became the *expected* value for the live lockfile. This is the
  third instance of the same shape (`vendor/`, `deps/`, now build staging), and directory-name blocklists cannot
  close it: `tmp`/`dist`/`build`/`stage` are per-tool names, whereas "not committed" is the property that
  actually separates a consumer's manifests from a build tool's copies. Both the `Cargo.toml` scan and the
  `Cargo.lock` check (and the `.csproj` scan) are now filtered to git-tracked paths; when git cannot answer (no
  work tree, no `git` binary) the previous unfiltered walk still runs, with a warning. The name-based exclusions
  stay for that fallback, and because a `vendor/` tree carried for offline builds is legitimately tracked.
- **csharp**: report unemitted visitor support files instead of deleting them. `generate_bindings` ran two
  `fs::remove_file` loops from inside the stage `collect_managed_surface` documents as "a pure in-memory render;
  nothing here writes to disk". `alef verify`, `alef adopt` — including without `--write`, since the delete fired
  before `AdoptOptions` was even constructed — and `alef diff` all compose that stage, so three read-only commands
  unlinked files in the consumer's tree, and for `verify`/`adopt` only on a cache miss. A filename match was the
  entire test, and the deleted set included a class per configured bridge `context_type`/`result_type`: names taken
  from the consumer's own config. The disabled branch was weaker still — `config.ffi` is an `Option`, so never
  having written an `[ffi]` section read identically to having disabled the feature.
- **c**: refuse to name a result type rather than inventing one. The old fallback derived it from the call name, and
  that name feeds three things — the accessor prefix, the `_free` cleanup, and the `parent_is_ir_type` flag
  `ensure_leaf_field_exists` reads. Because an invented name matches no IR type, that check returned `Ok` before
  examining anything: the fabrication switched off the check that would have caught the fabrication. Resolution now
  yields `Resolved` / `Unverified` / `Unresolvable` and fails at the point of emission. Trait-bridge registry calls
  (`register_fn` / `unregister_fn` / `clear_fn`) are classified from their derived C identity, not from their
  legitimately empty base `function`, which previously derived a degenerate `{prefix}__free`.
- **c**: stop splicing the fixture's `input` JSON into a typed parameter. With no configured `args` the emitter
  passed the whole JSON as one C string literal regardless of the target's signature, producing calls like
  `configure("{…}")` against a function taking an integer handle, and passing an argument to a `(void)` export.
  A zero-parameter target now emits `()`; a typed parameter `args` does not fill fails with a diagnostic naming the
  fixture, the call, the parameter and the config knob; an unresolvable signature refuses. Found independently in
  two repos on the same day — once in emitted output, once in the emitter.
- **cache**: treat an unreadable output manifest as a miss rather than a hit. `outputs_exist` returned `true` on any
  read error, so a cache with a missing or corrupt manifest reported a hit and skipped regeneration — and
  `write_lang_hash` writes the hash and the manifest as two separate writes, so an interrupted run leaves exactly
  that state. The ownership record now separates absent from unreadable: querying still degrades to "alef owns
  nothing", which makes the guard refuse and is non-destructive, while rewriting fails loud, because that path
  replaces the file whole and would otherwise silently un-own every path it could not read.
- **cache**: key the e2e stage cache by the `--lang` selection. A `--lang`-scoped run's partial output satisfied a
  later unscoped run, which then skipped the languages it had never generated.
- **release**: stop the version gate and `go-tag` from deciding on evidence they never read. `validate-versions`
  checked a manifest's existence and then discarded a failed read, so an unparseable `package.json` was dropped
  from the set and the gate reported "All N manifests consistent" over a smaller N, exiting 0. Zero checks is now
  an error rather than a vacuous pass, on both the text and JSON surfaces. `go-tag` ignored `git ls-remote`'s exit
  status, and a failed remote read yields empty stdout — indistinguishable from "tag absent" — so a transient auth
  or network failure created the tag and pushed it with `--force-with-lease`, which for a tag ref has no
  remote-tracking ref to lease against and degrades toward a plain force.
- **snippets**: release the client in generated Go, C# and WASM examples. Go defers `client.Free()` after the nil
  guard so it runs while a panic unwinds; C# uses a `using` declaration, which survives a throw; WASM uses
  `try`/`finally`. WASM releases only the client on purpose — every DTO-taking method already calls
  `__destroy_into_raw` on its argument and `free()` has no null guard, so releasing the request would have been a
  null-pointer free in most snippets, strictly worse than the leak it targeted. Java, Kotlin and Dart are staged
  separately.
- **csharp**: keep every vtable slot the Rust struct declares. C# carried the same unguarded `exclude_types` prune
  that cost Java a slot, and builds a positional `IntPtr` vtable, so a pruned trait method shifted every later
  function pointer and the last read ran past the allocation. Latent, but not for the assumed reason —
  `effective_exclude_types` draws from four sources, and `#[alef(skip)]` or `doc(hidden)` trips it with no
  `[crates.csharp]` config at all. A second defect ran the other way: `num_vtable_fields` never filtered
  `ffi_skip_methods`, emitting N slots into a struct declaring N−1.
- **csharp**: stop assigning `null` into non-nullable properties. 17 sites emitted `= default!` — compiles, the `!`
  silences the nullable analysis, first read throws. `CrawlConfig.Content` and `.Browser` are that case.
- **bindings**: stop substituting a zero for a default that was never read. Swift's memberwise init ignored
  defaults and decoded `Some(30)` as `nil`; Kotlin turned a function-call default on a collection into an empty
  one; the shared constructor path collapsed enum-variant and empty defaults into one `unwrap_or_default()`, so a
  `#[default]` variant other than the declared one shipped to wasm, pyo3 and extendr alike. pyo3's `None` was
  never the bug — three copies of a predicate matched only the bare `#[serde(default)]` spelling.
- **go**: send real defaults for a container-level `serde(default)` struct. `is_named_enum` shared the blind spot,
  and there it is worse — a unit enum's Go zero is `""`, never a valid variant.
- **e2e**: resolve result types from the IR on the generated-test-file path. `E2eCodegen::generate` carried no
  `functions`, so every IR lookup there was dead code. The invented type name was the visible symptom; the real
  cost was that a fabricated result type reports as not-an-IR-type, which switched off leaf-field verification on
  the very path it was written for.
- **e2e (ruby)**: assert streaming completion instead of dropping it. `stream_complete` and `no_chunks_after_done`
  matched a `None` accessor the resolver never returns, so both arms were unreachable — and the compensating
  assertion is suppressed precisely when a fixture asserts that field.

### Two invariants

Nearly every defect fixed below is one of two failures. They are worth more than the individual fixes, because the
fixes are local and the failures are not.

- **Absence from output is not evidence of intent to remove.** A value missing from a run's output has at least
  three causes: it was dropped on purpose, it was out of scope for this run (`--lang java` never computes the other
  languages), or it was never reached because an earlier step failed. Only the first licenses a removal, a prune or
  a skip, and no site in this release distinguished them. The worked example is the orphan reclaim gate: all five of
  its clauses — marker present, git-tracked, owned root, absent from this run's keep set, non-degenerate root
  manifest — passed for a consumer's 408-line Java public API class and for a class a live test depends on. The
  backend had run, emitted 56 files, and simply not emitted those two. The gate was introduced and disarmed inside
  this same release; the manifest-recording fix also in this release is what would have widened it, by making
  under-recorded manifests stop reading as degenerate. A second live delete path had the identical shape: the
  snippet prune ran ahead of the completeness gate its whole safety argument rests on, so a snippet that merely
  failed to render was unlinked as an orphan. The tell is an empty-collection fallback — `unwrap_or_default()`,
  `unwrap_or(&[])`, `Err(_) => continue` — whose result then drives a destructive or suppressive action. It is not
  confined to the generator: the cache, the verify pass and the release tooling all read an input they could not
  read as an input that was empty, and then report success.
- **One fact derived in two places, never compared.** Two emitters, or an emitter and a validator, or a backend and
  the docs renderer, each independently compute the same name, arity, nullability, sentinel or macro spelling. Each
  half is individually well-formed, so no compiler, linter or test observes a contradiction — only the composed
  output is wrong. This release closes such pairs in the C# capsule sentinel, the Zig declared error set, the Dart
  snippet call shape, Kotlin identifier escaping, pyo3 keyword fields, the cgo feature macros and `alef all`'s
  manifest. **The largest instance is still open**: `src/docs/naming.rs` renders Kotlin, Dart, Swift and PHP enum
  variants `to_pascal_case`, while the backends emit `to_screaming_snake` (Kotlin, `NETWORK_IDLE`),
  `to_lower_camel_case` (Dart and Swift, `networkIdle`) and `to_uppercase` (PHP, `NETWORKIDLE`). Every variant of
  every enum in four languages is documented under a spelling the binding does not export, with no configuration
  needed to trigger it.
  The fix direction is that the docs renderer must read the emitted binding rather than recompute it.
- **The remedy, which this codebase already applies well where it applies it at all.** Derive once rather than
  restate — `codegen::c_consumer::export_type_prefix` is the model. Where duplication is unavoidable, pin it with a
  test that drives *both* derivations and compares the composed output: `assert_error_set_covers_body` scans the Zig
  body that was just emitted, and the cgo feature-macro test parses the real generated `cbindgen.toml` rather than a
  copy of it. Enumerate from a registry rather than a hand-written list, and add a control assertion so the test
  cannot pass vacuously. A prose comment asserting that two modules agree cannot fail, and several such comments in
  this tree are false today; an assertion about a fact two modules share must name both sides, or it is a hostage
  rather than a guard.

### Upgrading

Read this before regenerating. Consumers pin `alef_version = "0.61.0"` in `alef.toml` today and 0.61.0 has never
been tagged, so those pipelines have been silently falling through to a branch build. Tagging resolves that pin for
the first time, which means the first resolved run is not a small step from whatever branch build preceded it.

0.60.2 was never tagged either — its `chore(release): 0.60.2` commit landed and the tag never followed, so the last
release any consumer could resolve is **0.60.1**. Upgrading from 0.60.1 therefore delivers the 0.60.2 section below
as well as this one; read both.

- **`--clean` no longer implies overwrite.** Until this release `--clean` was threaded into the `overwrite` argument
  of the scaffold and docs stages, disabling the create-only branch that otherwise leaves an already-existing
  unmarked file alone — so under `--clean` the ownership guard was the only remaining protection on a hand-written
  scaffold file. The two concerns are now separate: `--clean` bypasses cached results and nothing else, and
  overwriting a pre-existing unmarked scaffold or docs file is an explicit opt-in of its own. **Migration**: if your
  pipeline passes `--clean` only to force a fresh run, nothing changes and you gain back a protection you did not
  know you had lost. If it depends on `--clean` replacing pre-existing unmarked files, pass
  `--clobber-create-once-seeds` alongside it, or — better, because it is durable and reviewable — take ownership
  of those files once with `alef adopt <path>` and drop the flag.
- **Two large generated diffs land at once, from two unrelated causes. Review both for removals and for files that
  stopped being emitted; do not skim either as progress.** A large diff that reads as healthy is how the worst
  defects in this release stayed invisible. (1) The snippet-ownership fix unfreezes a generated snippet surface that
  nothing has been able to update for some time — on one consumer tree 2,820 of 2,894 writes were being refused — so
  the diff is the accumulated drift of every refused run, and it is the first diff in which a deletion is possible
  again. (2) The snippet resource-release fix separately rewrites a large number of published Go, C# and WASM
  examples, which previously leaked the client they construct. Seeing both at once is expected; they are two things.
- **Commit `.alef-ownership.toml` and `.alef-toml-merge-provenance.toml` if they are untracked.** `alef verify` now
  fails on this and prints the exact `git add` to run. See **Changed** for why an untracked record makes a green run
  on a warm machine certify a state no other checkout has.
- **The committed C header's content is now a function of the local build's feature set.** A `--no-default-features`
  build whose header is committed strips feature defines that a default build would have written, and nothing
  currently catches that. See **Changed**.

### Added

- **`alef adopt <path-or-glob>`**: take ownership of a pre-existing generated file so alef can regenerate it again.
  The write-time ownership guard refuses any pre-existing file it cannot prove it authored, which is correct but
  one-way: a file whose type became stampable only after it was committed carries no marker, so the write is
  refused, so the marker never lands, so it is refused forever. `crates/crawlberg-ffi/Cargo.toml` has never carried
  a marker in its entire history and had real fixes frozen out of it behind a warning nobody reads during a regen.
  `adopt` prints the full, untruncated diff between the file on disk and what alef would generate, and changes
  nothing without `--write`. Adoption stamps the marker onto the bytes already on disk and never writes generated
  content, so convergence happens on the next ordinary `alef generate`, in view of `git diff`. Formats that cannot
  carry a comment (`.json`) are adopted through the durable `.alef/` ownership record instead. It is deliberately
  not wired into `alef all` or any other command: no predicate over file content can tell "alef wrote this under an
  older release" apart from "someone hand-wrote this", because both are the same bytes.
- **config `[crates] cargo_lints`**: declare a per-crate `[lints.rust]`/`[lints.clippy]` table for generated binding
  crates. Previously inexpressible, so consumers hand-edited files headed `auto-generated by alef — DO NOT EDIT`
  and regeneration correctly restored declared content, silently making workspace `deny` levels inert in exactly
  the crates that most need them. `[lints] workspace = true` is not a substitute: all-or-nothing, and it pulls
  `unsafe_code = "deny"` which no FFI crate can accept. Covers the 11 Cargo.toml-emitting backends (ffi, node,
  python, php, jni, r, ruby, elixir, wasm, swift, dart); the six that emit no Cargo.toml are deliberately excluded.
  Where a backend already emits a builtin `[lints.rust]` block (elixir, swift, dart), a configured table merges
  into the same table rather than emitting a duplicate, which Cargo rejects.
- **`alef verify` reports frozen generated files**: a file alef intends to stamp, that exists on disk carrying no
  marker, can never be written by the ownership guard and so is frozen permanently. Previously this was invisible
  without running a generate. `collect_alef_hashes` cannot see such a file by construction — it only opens files
  that already carry a marker — so detection reuses the in-memory regeneration `verify` already performs to find
  missing files, intersected against what exists on disk unmarked. No heuristic and no added cost. Reported as its
  own section with the literal marker line to paste, never folded into stale or missing, because `alef generate`
  fixes those and cannot fix this. Scope is the ownership guard's freeze class only: create-once artifacts are
  emitted without a generated header and are deliberately excluded, so a hand-edited `Cargo.toml` is never reported.
- **eight create-once migrators**: artifacts emitted without a generated header are written once and never updated,
  so a repo scaffolded before a template fix keeps the broken content forever. Each migrator is pinned to the commit
  that fixed its template and repairs only content it can positively identify as alef's own stale output —
  `packages/dart/.pubignore`, `crates/*-wasm/package.json` exports, `crates/*-node/package.json` service export,
  `packages/java/checkstyle.xml` line length, the wasm `.cargo/config.toml` rustflags, and others. A migrator that
  cannot establish that provenance repairs nothing, because clobbering a hand-edit is far worse than a stale file.
- **fixture `docs.shows` gains `display`**: selects human-readable over debug formatting, matching the flag
  `iterate` already had. Without it Rust snippets always rendered `println!("{:?}", …)`, so documentation printed
  `Some(Text("Hello!"))` where a reader expects `Hello!`. Defaults to the previous behaviour.


- **readme**: expose each language README's generated public functions as structured `functions` template values
  with `name`, `rust_name`, `is_async`, and `documentation` fields. Names honor language exclusions, feature gates,
  ABI prefixes, Go type collisions, Ruby re-export names, and the centralized host-language naming policy.

- **`--version` carries build provenance**: the commit, the build time, and whether the tree was dirty. Three
  binaries built in one day all self-reported the same semver, and a defect that had already been fixed was
  investigated for hours against output from a binary that predated the fix. Generated output is evidence about the
  binary that produced it, not about the source, and until now nothing in that output identified the binary. A dirty
  build says so and names the commit it cannot be reproduced from; missing git metadata renders an explicit unknown
  rather than an empty field that would read as a clean build. `clean` is only as precise as the rerun-if-changed
  set; `dirty` and the sha are exact. **`--version` is now multi-line** — `-V` keeps the single bare semver line, so
  anything parsing a version out of `alef` should use `-V`.
- **breaking generated signatures are reported**: emitted public signatures are captured against a previous-run
  baseline, diffed, and each breaking change attributed to the callers that are not alef-owned. This warns rather
  than fails, because failing a regeneration on a change the consumer intends would be worse than the present
  silence; the value is the attribution. Zig only for now — other backends return no signatures, and a change
  detected for a language with no scan wiring warns rather than passing silently.
- **extraction warns when `#[serde(default…)]` and `impl Default` disagree for one field**: the effective value then
  depends on how the caller constructed the type, so a binding generated from one path silently contradicts the
  core when the other is taken. Alef read both and discarded the first — three writers assigned the same slot in
  sequence. The diagnostic stays silent unless both sides fold to fully concrete values: an `Unresolved` default
  means alef could not read a real `fn default()`, which is unknown rather than zero.

### Changed

- **`alef verify` now fails on a frozen file rather than warning.** This is a semantics change for consumer CI. The
  condition is permanent and self-perpetuating by construction — the guard refuses because there is no marker, and
  the marker can only arrive by writing the file — so no later run clears it and a warning is indistinguishable from
  one nobody reads. Remedy is `alef adopt <path>`.
- **the scaffold ownership record moved to a committed `.alef-ownership.toml`.** Ownership of a file whose format
  cannot carry a comment previously lived under `.alef/`, which alef itself writes into every consumer's
  `.gitignore` — so a fresh clone and a warm dev machine disagreed about which files alef owns, and CI refused
  writes a developer's machine permitted. Reads union the committed record with the legacy gitignored one, which is
  never written again, so upgrading does not turn every unmarkable file into a refusal at once. Entries migrate on
  the first authorised write of that path. Commit the file. Do not hand-add entries: use `alef adopt`.


- **formatting**: normalize the persistent FRB Cargo-cache helper with the repository Rust formatter.
- **test formatting**: normalize recently added FFI handle-registry, enum-conversion, and generated-hash regression
  fixtures with the repository formatter.

- **zig**: give each streaming adapter its own iterator struct type instead of naming it after the item type alone.
  Two streaming methods on the same opaque handle that yield the same item type (e.g. `crawl_stream` and
  `batch_crawl_stream`, both yielding `CrawlEvent`) collapsed into one shared `{ItemType}Stream` struct — whichever
  adapter emitted its struct first "won" the name, and the other adapter's wrapper method returned that same struct,
  whose `next()`/`deinit()` hardcode only the first adapter's `_next`/`_free` FFI symbols. That compiled and linked
  (both symbol sets exist in the C header) but handed the second adapter's stream handle to the first adapter's
  native functions — a runtime handle type-confusion bug, not a missing feature. Colliding adapters now each get a
  uniquely named struct derived from the adapter's own name. **Consumer-visible:** where a collision exists the
  shared `{ItemType}Stream` type is renamed to adapter-specific stream types. Zig code naming the old type explicitly
  must be updated. Types with a single streaming adapter
  keep their existing name.

- **write commands warn, and `alef verify` fails, when a required alef record is untracked.** Alef writes
  `.alef-ownership.toml` and `.alef-toml-merge-provenance.toml`, tells the reader to commit them, and never stages
  them; both were untracked on two consumer repos. Under `--clean` that record is the only protection left on a
  hand-written scaffold file, and it is the input the orphan scan reads, so a fresh clone has neither the protection
  nor a correct picture and a green run on a warm machine certifies a state no other checkout has. **Action: commit
  `.alef-ownership.toml` and `.alef-toml-merge-provenance.toml` if they are untracked in your repo.** `verify` prints
  the exact `git add` to run. Not auto-staged: mutating a user's index is a different licence from writing files, and
  in CI it would accomplish nothing.
- **the generated `build.rs` stamps a `#define` per `CARGO_FEATURE_*` into the C header.** cbindgen guards a
  declaration for every `#[cfg(feature)]` export, deriving the guards from the unfiltered API surface, while the Go
  glue is generated from the cfg-filtered one and the cgo preamble recorded neither — so the guarded declarations
  were exactly the ones the glue calls, and nothing anywhere defined the macros: 62 of 144 symbols invisible on one
  consumer, the whole C snippet lane dead on another. The defines are written after cbindgen emits the header, from
  literally what the cdylib was built with, so there is no second derivation to drift, and they reach Go, C snippets,
  the e2e Makefile and Zig at once. Guards stay load-bearing for a slim build: a disabled feature gets neither a call
  site nor a define. **Cost: the committed header's content is now a function of the local build's feature set.** A
  `--no-default-features` build whose header is committed strips defines that a default build would have written, and
  nothing currently catches that.
- **the snippet coverage ledger now proves alef's ownership of a generated snippet.** A snippet population predating
  marker support could never be updated — the write guard refuses a pre-existing file it cannot prove it authored,
  and the marker that would prove it is what the refused write was going to add. On two consumer trees that froze the
  entire generated snippet surface: 2,820 of 2,894 refusals on one. The ledger is a record rather than an inference,
  it is committed so a fresh clone behaves identically, and alef already unlinks files on its strength, so refusing
  to overwrite what it will happily delete was incoherent. The snapshot is taken before generation, or this run's own
  intentions would widen ownership to bare path identity. The first successful regeneration after upgrading produces
  a large snippet diff; see **Upgrading** for what to check in it.
- **kotlin**: hard keywords are escaped in every identifier path. Identifiers went through three parallel paths — one
  escaped nothing, one ran the escape on an already-PascalCased string so no all-lowercase keyword could match, and
  only the field-name path worked, so `fun object(...)`, `fun is(...)` and `when: String` reached consumers as parse
  errors. Backticks rather than renames: the DTO emitter writes `@JsonProperty` only when the Rust field carries
  `serde(rename)`, so renaming would silently move the wire key for every unrenamed field. `get`/`set` stay bare on
  purpose — they are soft keywords the grammar admits as `simpleIdentifier`.
- **pyo3**: one Python-visible name per keyword field. Five producers computed five different names for a single
  field: the getter published the bare keyword, the converter emitted `_rust.T(global=value.global)`, and a keyword
  field got no wire alias at all, shipping the escaped spelling as the JSON key. The escape now reaches the Python
  surface only; serde keeps the wire name and prefers the core type's own rename over deriving one from the escaped
  field. Where the Rust and Python escapes collide the Rust one wins — pyo3 strips `r#` when deriving the Python
  name, so `r#type` satisfies both while `type_` satisfies neither.
- **dart**: the trait emitter was the only Dart path calling `to_lower_camel_case` without the keyword escape, so
  `new` and `get` — the two commonest Rust trait method names — were emitted bare. They are now escaped, which
  renames them on the generated Dart trait surface.

### Fixed

- **go**: stop asserting a wire shape serde does not produce. Go encoded every `std::time::Duration` field with
  the `DurationMillis` helper, which writes serde's derived `{"secs":_,"nanos":_}` object. A field carrying
  `#[serde(with = "…")]` has a hand-written codec expecting a bare millisecond integer, so the derived shape is
  wrong for it and every config construction round-tripping through JSON failed with `invalid type: map, expected
  u64`. The IR could not tell the two apart — the extractor implemented five serde readers and none read
  `with`/`serialize_with` — so `FieldDef::serde_with` now records it. Reading all occurrences matters:
  `deserialize_with = "…"` contains `serialize_with = "…"` as a substring, so a first-match-only scan silently
  picks the read side. `api_has_duration_field` moves in lockstep, otherwise a crate whose only Duration fields are
  hand-coded gets an unused helper and an unused `encoding/json` import, which Go rejects.
- **java**: keep a vtable slot for a trait method returning an excluded type. `api_without_excluded_types` lost its
  `if !typ.is_trait` guard as incidental cleanup, so a bridge method whose signature named an `exclude_types` entry
  was pruned from the Java surface while the Rust vtable still declared its slot. Every function pointer after the
  dropped one then dispatched to the wrong method, with the last read running past the end of the struct. The prune
  is silent because it removes the method from the interface and adapter too, leaving all three Java files mutually
  consistent. Both emitters now take the slot list and its ABI order from one function, and generation fails when
  the declared and emitted layouts disagree. C# has the same unguarded prune and a positional vtable of its own; it
  is latent only because no consumer sets `[crates.csharp] exclude_types`.
- **bindings**: restore struct defaults the emitters dropped. Java decided twice, from two independent lists,
  whether a field carries a literal default — one governed boxing, the other the compact-constructor restore — and
  only the second was ever extended, so float defaults crossed the wire as `0.0`. Kotlin rendered a `f64` default of
  `1.0` as `Double = 1`, which does not compile, and emitted bare `NaN`/`inf`. Rustler decided what kind of default
  it had by sniffing rendered Rust text for `::` or a leading quote, collapsing every string default to `""`. All
  three now ask `typed_default`, and Java and Kotlin share one float renderer. **Breaking for Java consumers**: a
  `boolean` component carrying a `true` default becomes `Boolean`, since boxing is the only way to distinguish
  "not supplied" from the type-zero. Boxing applies only where the default differs from the zero.
- **e2e (C)**: reject an assertion whose leaf field the IR does not declare. The nested-accessor walk validated
  every intermediate hop and nothing at the leaf, defaulting to `char*` and synthesising `{parent}_{field}` as the
  symbol. Generation reported success and the failure surfaced only at `cc` time. Three existing mechanisms could
  not see it: `is_valid_for_result` is head-only by construction, splitting on `.` and inspecting only the first
  segment; the unavailable-field scanner looks for a comment this path never writes; and
  `ALEF_E2E_STRICT_FIELD_AVAILABILITY`, which arms the markers that are written, is set in no repository.
- **e2e**: resolve call names through the override chain. `CallConfig::function` is legitimately empty when a call
  names itself only per language, so sixteen sites reading it directly failed silently — adapter and IR lookups,
  `request_type`/`streaming_item_type` keys, and a `returns_void` classifier that bound a result from a void C#
  method on every registry call. Two resolvers now cover the two distinct questions, and a structural guard pins
  every remaining raw read against an allowlist keyed by source text rather than line number.
- **e2e (C, Zig)**: derive the C export prefix once. cbindgen writes `[export] prefix` as shouty-snake while every
  C and Zig emitter re-derived it with `to_uppercase`; the two diverge for any prefix with an internal word
  boundary, naming types absent from the header the snippet compiles against. Zig snippets also ran the closing
  `std.debug.print` onto the previous line.
- **snippets**: report per-language failures instead of one number. A run with 1753 failures emitted a start line, a
  finish line and a summary count, so a language failing every snippet was indistinguishable from one that passed.
  Java failed for an unrelated reason: session scratch moved outside the Maven source root, but nothing swept what
  older versions had written, and the leftovers are self-perpetuating — the consumer's own `mvn package` hook hits
  `duplicate class` and fails preparation for the whole language on every future run. The sweep is unconditional
  rather than tied to `--clean`, since needing a flag to get a correct run is a workaround.
- **snippets**: keep the mock harness out of published examples. C streaming and byte-buffer snippets published
  `create_client("test-key", NULL, …)` — a literal harness credential with no environment read. Unlike the other
  leaks this one was not blocked, because that string is not in the guard's marker list. Swift emitted the
  environment-reading constructor only when a fixture named a credential variable, and Elixir snippets called the
  module directly although the exported arity includes the client, naming a function that does not exist.
- **docs**: strip every `~keep` spelling. The stripper removed the marker's five bytes then chose between eating
  the following whitespace or one preceding character, so every variant with attached punctuation stranded it:
  `~keep:` left `.:`, `~keep,` left `.,`, and `(~keep)` — the most common broken form — left empty parens.
- **e2e (python)**: define the helpers the generated file calls. `_alef_e2e_text` has two independent callers but a
  single gate keyed on the second emitted both definitions, so a file whose only caller was the enum equality
  assertion shipped 22 undefined names.
- **codegen**: one snake-caser and one attribute scanner, not three. `error_gen` carried a third caser splitting
  before every uppercase letter, so `GraphQLError` became `graph_q_l_error` in C accessor symbols while the repo's
  declared derivation produced `graph_ql_error`; a fourth caser for screaming-snake shared the flaw, and one
  generated snippet pairs the two. `rust_type_kind_hints` reset its state on any line not starting an attribute, so
  a rustfmt-wrapped `#[derive(…)]` between a `#[repr(…)]` and its struct discarded the hint.
- **e2e/snippets (wasm)**: stop gating snippet availability on a codegen predicate. `function_is_exported`
  answers "should the plain-function generator emit a wrapper for this?" and returns `false` for trait-bridge
  register/unregister/clear functions precisely because the trait-bridge generator emits them instead. The snippet
  gate reused it to mean "can a snippet call this?", where `false` is flatly wrong — those functions are exported,
  from the generated `__alef_wasm_bridge_*` modules. A 0.61.0 regression with a committed positive control: 0.60.0
  emitted a valid snippet for the same fixture, with the correct import and JS name. It did not merely drop
  snippets, it aborted `alef all` before a byte was written, so an affected repo could not regenerate at all.
  Symbol resolution is fixed alongside the predicate: the gate needs a Rust identity while
  `overrides.wasm.function` legitimately holds the JavaScript spelling, so a symbol now resolves under either
  spelling and the bridge registry is searched beside the plain function surface. A name that resolves to nothing
  is reported as its own condition — folding it into "not exported" sends the reader to audit the wasm backend for
  what was only ever a misspelling in config.
- **cli/generate**: read a `#[cfg(...)]` attribute that rustfmt wrapped. The FFI header parity gate scanned
  attributes one line at a time, so a wrapped predicate was lost twice over — the single-line parser failed on the
  opening line, and the continuation lines, being neither attribute-prefixed nor function signatures, hit the state
  reset that clears the pending cfg. The export was recorded as unconditional and a correctly guarded header was
  reported as drifted, aborting `alef all`. The gate was unclearable by its own remedy: it advises running a cargo
  build so cbindgen regenerates the header, and the run had already done exactly that, successfully, moments
  before. An attribute the scanner cannot delimit is now recorded as an unparsed cfg rather than as no cfg, so the
  gate gets stricter here rather than looser.
- **cli/generate**: report every refused write rather than one phase's. A run writes through five independent
  phases, but the consolidated refusal summary was emitted from inside the scaffold writer, so it could only ever
  describe scaffolding — and `alef all` never called the reporter at all, dropping `refused_paths` from its four
  write sites. Measured across three real regenerations: 34 refused and 30 reported, 15,677 refused and 15,669
  rostered, 34 refused and 29 reported. The omitted paths are real binding sources. Worse than a wrong number: the
  summary tells the operator to review and adopt each path, so someone who works the printed list finishes
  believing they are done while those files stay permanently frozen.
- **backends/zig**: never report a typed error the binding cannot substantiate. A variant's FFI code comes only
  from an explicit `#[alef(error_code = N)]`, and no consumer declares one, so every zig binding resolved every
  failure to `_first_error(E)` — literally the first declared variant. Silently wrong error types rather than
  missing ones. The FFI layer was already honest here, sending `ALEF_FFI_UNKNOWN_ERROR` across the boundary; zig
  was the only backend that turned unknown into wrong. `_first_error` is removed rather than patched, because eight
  further emission sites used it with the identical defect — null constructor handle, null `to_json` pointer,
  stream start and next, trait-bridge clear and unregister, opaque-handle returns. An implicit `UnknownFfiError` is
  injected into every generated error set by the same mechanism that already injects `OutOfMemory`, so it coerces
  into any caller-supplied set; coded variants still dispatch per code, and only the `else` arm changed. Separately,
  `_first_error(anyerror)` never compiled at all: `@typeInfo(anyerror).error_set` is `null`, so `orelse unreachable`
  was comptime-evaluated, and two templates emitted exactly that. Five assertions across two files pinned the old
  behaviour — including one named `..._use_the_unknown_fallback` that asserted `_first_error`, which is not an
  unknown fallback — and were inverted rather than worked around.
- **e2e/snippets**: give generated markdown a provenance marker. Fixture snippets carried neither a marker nor an
  ownership record, so once written they were refused forever — 15,677 refusals in one consumer repo and 9,139 in
  another, dominated by the ~12,000 snippet `.md` files between them. `marker_header_syntax` excludes `.md` on the
  stated grounds that `readme::template` and `docs::render` both route content through
  `docs::render::with_html_header`; that is true of READMEs and docs pages and false of fixture snippets, which are
  assembled in `render_snippet_markdown` and never touch `docs::render`. The header now comes from that same
  emitter rather than a second producer. `marker_comment_style` is untouched — `.md` stays out of the *ownership*
  predicate, since adding it there would freeze every unmarked `.md` in every consumer repo. Note the placement has
  zero slack: front matter is 8 lines, the header lands on line 10, and the marker scan window is 10, so a ninth
  front-matter field would silently restore the deadlock with the marker still in the file. Files already committed
  without a marker are not unfrozen by this and still need adoption or regeneration from absent.
- **scaffold/ownership**: stop minting ownership from byte-equality alone. Four write paths recorded a file as
  alef-owned whenever its bytes already matched generated output, *before* any ownership check ran — the rejected
  content-equivalence predicate, relocated from a predicate into the record. A hand-written file that coincided with
  generated output silently acquired permanent overwrite permission, and the run that granted it changed nothing
  observable, which is why a test asserting only on file contents and the changed count passed throughout.
  Ownership is a fact about history, not about content; only `alef adopt` confers it now.
- **docs/c**: derive every documented C symbol from one helper. Docs published `{prefix}_{method}` while the FFI
  backend emits `{prefix}_{type_snake}_{method}`, so documented symbols did not exist, and the `this: AlefHandle`
  receiver was missing from documented signatures. A repo sweep found the symbol shape derived at roughly 262 sites
  — four of them independently inside docs alone — so patching the docs arm would have traded one divergence for
  two. Producers, docs and the streaming `_start`/`_next`/`_free` triple now route through `free_function_symbol` /
  `method_symbol` / `stream_adapter_symbol` in `codegen::c_consumer`.
- **backends/java, backends/kotlin**: reject a service `Finalize` entrypoint whose return type is not opaque. The
  FFI layer renders every non-opaque entrypoint return as an `i32` status code — `null_return = 1` on the error path
  — and no template carries a primitive value across the boundary. Java's representability gate nevertheless
  admitted the shape and then emitted `void`, and Kotlin would have declared `Int` against that `void`. Generation
  now fails on the shape instead, and Kotlin's `Finalize` return is the raw `AlefHandle` as `Long`.
- **scaffold/zig**: emit no test step when there is nothing to assert. `zig build test` exited 0 having run nothing,
  which is indistinguishable from coverage. The seed file and the `test_module`/`test_step` block now branch on one
  condition, so the step and the file it points at cannot drift apart; an empty surface fails with
  `error: no step named 'test'`. The fixture that should have caught this was empty, so the test validating the seed
  was validating the placeholder.
- **e2e/snippets**: keep mock-harness scaffolding out of published documentation. The zig snippet renderer called
  the test renderer, so published docs told readers to read `MOCK_SERVER_URL` and route through `/fixtures/<id>`.
  The existing scrub never covered this surface in any language — it swaps a placeholder in `fixture.input`, while
  these URLs are synthesised by the client-factory emitters from the fixture id at emission time. The guard is now
  applied at the single funnel every language and extension passes through, so a new backend inherits it.
- **e2e/codegen/rust**: emit an error branch for a fixture that expects an error. Rust was the only one of the
  fifteen snippet languages without one; it rendered `.expect("call failed")`, so the snippet documenting a failure
  panicked on it. Rust has no `try`, so this is a `match` on the `Result` with an `Err` arm.
- **hooks/check_project_mentions**: count `#[cfg(test)]` braces in code rather than in raw text. alef is a template
  generator and its comments are dense with Jinja — `{% endif %}` alone is one opener and two closers — so prose
  steered the exemption. Surplus closers ended the region early and reported test fixtures as violations; surplus
  openers extended a phantom region past the module's closing brace and silently hid every real violation in the
  production code below, leaving the gate reporting clean. Both directions are covered by a regression test.
- **backends/swift**: replace five regression files that had never been compiled. None was in the module tree, so
  Rust never saw them, and all sixteen of their assertions were `assert!(true)`. Wiring them in as written would
  have added no coverage. Rewritten against the private modules they actually cover.
- **bin_cli/verify**: scan every backend's output when collecting `alef:hash:` provenance. `VERIFY_SCAN_EXTENSIONS`
  omitted five backends outright, so `alef verify` reported those trees clean without ever opening a file in them —
  a passing verify was evidence of nothing for the languages it silently skipped. Dotfile stamps
  (`.gitignore`, `.gitattributes`, `.editorconfig`) were unreachable by construction on top of that, since
  `Path::extension` returns `None` for a name that is entirely a leading-dot stem; they are now matched by filename.
  The test that should have caught this was itself vacuous — its fixture wrote a stamp line but no hash line, and
  `collect_alef_hashes` requires both, so the sibling positive control asserted over an empty set. The fixture now
  emits a real stamped-and-hashed file, and a guard test pins that the walk actually collects what the fixture writes.

- **readme**: honour a configured `output_path`/`output_pattern` on the hardcoded fallback route, not only the
  templated one. `try_render_configured_readme` returns `None` in five distinct situations — no `template_dir`, a
  `template_dir` that does not exist, no entry for the language, no legacy YAML entry, or an entry whose template
  file is absent — and each one discarded the configured path along with the template, because path selection was
  reachable only from inside the templated route. A configured output path is not a property of a template and
  survives all five. Both routes now apply one precedence rule; the fallback previously composed its own, and the
  defect went unnoticed because the derived path usually agrees with the configured one. Agreement was coincidence.

- **backends/php**: emit `from_json` and the flat-field accessors in `.phpstub` output for tagged data enums. The
  runtime emits `from_json` unconditionally for every such enum plus a readonly property per flat field, while the
  stub declared neither — six enums in one consumer repo, three in another, two of them rendering as literally
  empty classes. The stub gate is `is_tagged_data_enum` alone and deliberately *not* the crate-level serde probe
  used for structs: the flat-enum mirror's serde derives are hardcoded in its template rather than gated, so keying
  the stub on the probe would reintroduce the same divergence in serde-less crates. Accessors are declared as
  properties rather than methods because ext-php-rs registers `#[php(getter)] fn get_x` as a property named `x`
  with no case conversion — the inverse of the struct path, which emits plain methods that land as `getX()`.

- **backends/magnus**: derive the async return annotation from one fact instead of three hand-maintained copies.
  `function_async_body.rs.jinja` is a single template serving both `has_error` arms and opens with a fallible
  `Runtime::new()?` in each, so building the tokio runtime makes an async binding fallible regardless of what the
  Rust signature declares. Two annotation sites disagreed with that: one hand-recomputed a subset of the `has_error`
  local already in scope four lines above it, dropping the `is_async` and `force_result_for_deser` terms.

- **docs**: stop emitting a Java constructor name the backend never generates. The docs carried two hand-written
  copies of the keyword-rename table, both wrong — they mapped `default` to `defaultOptions` and had no `new` arm
  at all, so an opaque type's default constructor reached `assert_valid_identifier` as the Java reserved word `new`
  and panicked, aborting the docs run. The table is now mirrored from the backend's `safe_java_method_name` with a
  test pinning the two together, and the duplicate copy is gone. Also corrects a cluster of `~keep` comments that
  asserted `Language::Jni` was unreachable — it is reachable today, and the files cited as proof say the opposite —
  and renames five tests whose names claimed the output had once been correct for a backend it never suited.

- **e2e/codegen, backends**: fail at generation time instead of emitting placeholder values that let a generated
  suite pass while testing nothing. Ten sites across eight backends silently fabricated output: the Ruby extension
  returned the literal `"[unimplemented: <fn>]"` (and `0`/`false`) for non-delegable functions, PHP, wasm-bindgen
  and the Elixir NIF did the same, `dart:ffi` mode dropped async functions from the API surface behind a comment,
  and the Rust e2e streaming path emitted nothing at all through unguarded `if let` arms. Two were worse than
  vacuous: the pyo3 capsule spliced `unreachable!()` into a value position, which compiles and then uncatchably
  panics the interpreter on first call, and the JNI `nativeRegister<Trait>` shim accepted a registration without
  ever calling `register_fn`, reporting success for a backend it never registered. Each now fails loudly —
  `compile_error!` where the backend already had that escape hatch, an explicit panic naming the crate, fixture
  and symbol elsewhere — and the pyo3 case raises a catchable `PyRuntimeError`.

- **e2e/fields**: derive field availability from the IR rather than from the hand-maintained `result_fields` TOML
  list, across fourteen backends. The list was wrong in both directions simultaneously in a real consumer repo —
  omitting a field that is exposed and listing one that a getter also exposes — so assertions were silently
  replaced by `skipped: field not available` comments. `FieldDef.binding_excluded` is now consulted first; it is
  not a proxy, being the same predicate `binding_fields()` uses to decide which fields the pyo3 backend gives a
  getter. Config and its sibling maps remain a fallback for names the IR has never seen. Note `#[serde(skip)]`
  never implies binding exclusion — only `#[doc(hidden)]`, `#[cfg_attr(alef, alef(skip))]` and `dyn Trait` fields
  do — and a control test now pins that, since a field can be absent from the wire format and still be exposed.

- **backends/rustler**: bound the visitor bridge's reply wait. `visitor_send_and_wait` blocked on `rx.recv()` with
  no timeout, so a host process that exited before replying held the NIF scheduler thread indefinitely; the
  trait-call path already had a watchdog and it was simply never retrofitted. The visitor channel carries no error
  slot, so the watchdog drops the sender and the existing disconnect path returns the method's default result,
  matching what the trait path already does on a closed channel.

- **backends/java**: resolve the libc `free` symbol lazily inside `freeHandlerResponse` instead of eagerly in the
  service class's static initializer. `SymbolLookup.loaderLookup()` only sees symbols reachable through libraries
  the classloader has loaded, and `free` is not emitted by alef's own FFI, so a service that never frees a handler
  response could fail to load at all. The sibling `malloc` lookup was already lazy; the asymmetry was the defect.

- **backends/zig**: emit the unwrap expression for a method or function returning `Option<OpaqueHandle>`. The match
  had arms for every other shape, so `Optional(Named)` fell through to a catch-all that returned the raw C value
  while the signature declared `?TypeName` — type-incorrect code that the vacuous generated test target concealed.

- **cli/generate**: skip languages with no binding backend instead of panicking. The build path already guarded
  this for docs-only targets (Rust, C); the generate path called `get_backend` unguarded on the same input.

- **scaffold/swift**: seed the generated test file with a real assertion. It emitted `XCTAssertTrue(true)`, which
  compiles and proves nothing; it now round-trips a serde DTO through `JSONEncoder`/`JSONDecoder` where the API
  surface allows, falls back to a type-resolution check, and keeps the bare placeholder only for an empty surface.

- **snippets/java**: write the validation scratch session outside the Maven source root. The generated `pom.xml`
  sets `sourceDirectory` to the project basedir, so the compiler plugin's `**/*.java` glob swept scratch snippet
  sources into the consumer's own build. `target/` is not a safe alternative for the same reason.

- **docs**: read signature contracts from the emitted binding instead of recomputing them per language. Java
  signatures stated the wrong `throws` contract on every method, so every rendered example failed to compile;
  Dart dropped the `Future<>` wrapper that its whole binding carries, and rendered optional parameters in
  positional rather than named syntax; Elixir dropped the receiver an instance function actually takes. Rust
  `Default`/`Clone` derives were documented as public API in languages that emit neither, error sections listed
  Rust enum variants rather than the generated exception classes, and integer and `Option` types were computed by
  a per-language formula that no two backends agreed on. An explicitly overridden return type is now authoritative
  and is no longer re-wrapped, which had turned a streaming `Stream<T>` into `Future<Stream<T>>`.

- **Java service bindings**: retain paired callback response deallocators and registration variant metadata while
  leasing service owners, and omit public functions whose signatures reference excluded types.

- **snippets/strict**: stop counting an explicit front-matter `level:` as a downgrade. `level:` is a validation
  contract — the author asked for exactly that level — and used to collapse into the same internal field a `<!--
  snippet:*-only -->` suppression comment uses, so a snippet that got exactly the level it declared was reported as a
  `strict`-failing `Downgraded` violation identical to one that suppressed validation below what the run requested. A
  declared `level:` that is fully honored now passes and carries a `Declared` `downgrade_reason`; a suppression
  annotation is unchanged and still fails strict, as does a declared level the environment or validator cannot
  actually reach.
- **snippets/strict**: extend the `max_level` capability-ceiling exemption to a validator's structural
  `achievable_level` gap. `php`, `ruby`, `elixir`, `bash`, and `r` cap `typecheck` down to `syntax` unconditionally —
  no checker is wired up for any of them, on any machine — but only `max_level` was exempted from `Downgraded`, so
  `validation_level = "typecheck"` plus `strict` was structurally unsatisfiable for any repo containing one of these
  languages, however healthy the environment was. Validators now declare whether an `achievable_level` gap is
  structural (permanent, exempted like `max_level`) or environmental (this run's machine only, e.g. a missing
  type-checker binary — still a genuine `Downgraded`); the five listed above declare theirs structural, and their
  affected snippets now report a capability-capped `Pass` instead of failing strict.
- **snippets/strict**: surface hard failures and session/preparation errors ahead of a strict downgrade bail, and
  name the results behind every strict-mode failure count. A run carrying both real `Fail`/`Error` results and
  `Downgraded` results previously reported only the downgrade count, because the strict downgraded check ran and
  bailed before the failure check further down was ever reached — a consumer investigating "N downgraded" never
  learned the run had failed outright. Every `ValidationResult` now carries a `downgrade_reason`
  (`Declared`/`Annotation`/`ValidatorCapability`/`Environment`), and the strict-failure and capability-capped-warning
  messages group by that reason as well as by language, so a consumer sees *why* a level differs, not just that it
  does.
- **snippets/sessions**: log a `tracing::error!` naming the target and language whenever
  `prepare_sessions_isolated` fails to prepare a validation session. Every snippet aimed at a failed target silently
  became a `SnippetStatus::Error` downstream with no other signal that the *target*, not the individual snippets, was
  what broke — this module had no `tracing::` calls at all before.
- **snippets/batching**: make the batch/fallback dispatch path observable. A language whose validator never overrides
  `validate_batch_in_session` (the default implementation always returns `None`) was still logged
  `Starting batched snippet validation`, then silently fell through to the per-snippet fallback with no matching
  `Finished` event — an observability gap in the dispatch path itself, not a signal about whether the validator ran
  or hung (a healthy, fully-passing language was exactly as silent there as a broken one). Validators now declare
  `supports_batching` upfront so a non-batching group never enters the batch codepath at all; a validator that does
  support batching but declines a specific group (rust declining to batch `Run`-level snippets) now logs an explicit
  fallback notice instead of a silent `continue`; and the per-snippet fallback dispatch itself now logs its own
  `language`-tagged `Starting`/`Finished` pair per language with a count and duration, so a `Starting`/`Finished`
  correlation by name works the same way for a fallback language as it does for a real batch. `run_validation` also
  now re-enters the caller's tracing span inside the rayon thread-pool closure, since `ThreadPool::install` always
  runs on a pool worker thread and span context is thread-local, not inherited across that boundary.
- **snippets/sessions**: purge stray top-level files left in a session's persistent `workspace_directory` (java,
  csharp, typescript) before running configured `before` hooks. That directory is deliberately reused across every
  snippet in a session and across every future run with an unchanged fingerprint, so compiled-artifact caches in its
  subdirectories survive between runs — but nothing ever removed the scratch source file each snippet's validate call
  writes at its top level, so it accumulated one leftover file per distinct snippet ever validated under that
  fingerprint. A consumer-configured `before` command that builds the whole module from `working_directory` (`mvn
  package`, for a Java session) runs once per session, before any of that run's own snippets are written, so the only
  way it could trip over bad scratch content was a leftover from a *previous* run — and one bad leftover then failed
  session preparation and stamped every snippet in the session `SnippetStatus::Error`, turning one bad snippet into
  an entire language going dark.
- **docs**: stop discarding every already-rendered API reference page when a later docs-stage step fails.
  `generate_docs_stage` renders the 15+ `api-*.md` pages plus `configuration.md`/`types.md`/`errors.md` before
  snippet discovery, snippet validation, CLI/MCP adoption checks, or llms/skills rendering ever run, but returned a
  single `Result<Vec<GeneratedFile>>` — so a failure in any of those later, unrelated steps (a strict snippet
  validation bail, an unmanaged `llms.txt`, a missing `docs.snippets.dirs` root) discarded the whole `Vec` and wrote
  nothing at all. A single strict-mode snippet failure could therefore silently freeze the entire published API
  reference at whatever version last validated cleanly, with no signal to the caller that anything was skipped.
  `generate_docs_stage` now returns `(Vec<GeneratedFile>, anyhow::Result<()>)`: callers write the pages unconditionally
  and only then propagate the error.
- **scaffold**: fix `detect_workspace_inheritance`, which never detected anything. It used
  `contents.parse::<toml::Value>()`, but `toml` 1.x's `FromStr for Value` parses a bare *value*, not a document, so
  it failed at `[workspace]` on every real Cargo.toml and silently returned an all-false result. Every binding-crate
  emitter that consults it (ffi, php, ruby, node, python, dart) therefore emitted a literal `version = "…"` instead
  of `version.workspace = true`, and likewise dropped `readme`/`keywords`/`categories`/`license` inheritance, so
  generated crates drifted behind workspace-wide bumps. The same mistake in the elixir scaffold silently yielded an
  empty feature list.
- **ffi/cbindgen**: emit `[defines]` feature keys unquoted — the `format!` sat inside a raw string so the key
  carried literal backslashes; cbindgen's `DefineKey::load` splits on `=` and trims but never unquotes, so no `#if`
  guard was emitted for any feature-gated export.
- **cli/build**: run the `ffi_dependent` stage even when an earlier independent group fails. The result loops used
  `let (stdout, stderr) = result?;`, returning on first failure and making that entire stage — go, java, csharp,
  kotlin_android, zig, jni — structurally unreachable.
- **cli/build**: supply default build recipes for swift, zig and gleam; `build_command_for` had no arms for them
  and fell through to `_ => "false"`.
- **registry**: return an error instead of panicking for `Language::C`; listing `"c"` in `[workspace] languages`
  aborted the run.
- **codegen/errors**: stop interpolating the error code into the exception message (pyo3 and napi), violating a
  documented invariant in the same file; the code now travels through a structured channel (`code: u32` on the
  generated Info classes). Note the leaked value is consumer-dependent — a repo that allocates no codes leaks an
  UNKNOWN sentinel uniformly, one that allocates leaks real codes — so the regression test asserts absence of any
  `[N]` prefix rather than a particular value.
- **codegen/errors**: restore newline separation between generated match arms; a nested `{%- if %}` inside
  `{%- for %}` collapsed the whole `match` body onto one line.
- **scaffold/ownership**: normalise the ownership-manifest key against `base_dir`. Callers disagreed on spelling —
  most commands pass an absolute `current_dir()`, the version-regen helpers pass `PathBuf::from(".")` — so the same
  file produced two keys and ownership established by one command was invisible to another.
- **scaffold/poly**: dedupe managed TOML arrays by decoded value rather than serialized text, and prune entries
  alef itself previously generated and no longer does. Pruning is provenance-gated so consumer-authored entries are
  never removed.
- **readme**: embed a provenance marker in generated READMEs so regeneration no longer depends on gitignored
  machine-local state.
- **docs/c**: document the real C ABI — handles render as the scalar handle type rather than invented per-type
  struct pointers, `bool` renders `int32_t`, a fallible void-returning function documents its `int32_t` status
  return, optional parameters no longer gain a second `*`, the error phrase is selected by return shape (`-1` /
  handle `0` / `NULL` / numeric `0`) instead of a blanket `NULL` claim, and every C type page states that the type
  name is documentation-only.
- **docs/go**: render static methods without a receiver, and pointer-wrap `Named` returns to match what the Go
  backend actually emits.
- **docs**: reject reserved words and malformed identifiers in generated signatures. The docs pipeline emitted a
  `new` constructor uniformly across languages with no check that the token was a legal identifier there — in Java
  and Dart `new` is reserved, so the documented signature was not parseable source.
- **e2e/tests**: emit a real assertion for a fixture whose only assertion is `not_error`, across python, php, java,
  csharp, swift, dart, elixir and typescript. Each backend treated `not_error` as needing no statement — correct in
  isolation — but when it was the only assertion the result was discarded entirely, and php additionally emitted
  `expectNotToPerformAssertions()`, which suppresses PHPUnit's own risky-test detector. Also stop emitting assertion
  helpers into files that never reference them, and derive the "has a usable assertion" decision from rendered
  content instead of a separately-maintained predicate that could drift.
- **e2e/kotlin, e2e/c**: fail generation instead of splicing a placeholder into an argument list. An unimplemented
  `TestBackendEmission` carries an `arg_expr` of literal comment text, which was pushed into the positional argument
  list unchecked; an unregistered trait bridge pushed a bare `null`.
- **backends/go**: derive snippet call shape from the extracted `FunctionDef` instead of re-asserting it in
  configuration, and infer `ptr(N)`'s type from the destination field rather than the literal.
- **backends/java**: serialize `Duration` fields as the real wire shape via paired Jackson converters, on both the
  record component and the builder setter.
- **backends/zig**: wrap an optional capsule return (`emit_function` matched only `TypeRef::Named`), use the
  correctly prefixed visitor-callbacks struct name, and back `VisitorHandle` with `u64` rather than `*anyopaque`.
- **snippets/zig**: emit a relative dependency path and a `fingerprint` in the generated `build.zig.zon`; Zig
  rejects an absolute `.path` outright.
- **backends/php**: sort constructor parameters required-before-optional in the runtime binding to match the stub,
  and apply the stub's `Duration` widening so the two agree on type and nullability.
- **backends/php, backends/extendr**: stop suppressing a generated enum variant factory when its name collides with
  an `enum_def.methods` entry; no backend forwards those methods into generated output, so the suppression dropped
  the factory with nothing replacing it.
- **backends/magnus, backends/php, backends/napi**: stop declaring in stubs what the binding generator does not
  emit. Each had a declaration generator and a binding generator independently deciding what exists; the stub side
  now consults the binding side's own predicate.
- **extract/reexports**: AND-combine a re-export or module `cfg` with an item's own instead of filling only when
  absent, so a type behind `#[cfg(feature = "a")]` re-exported through a `#[cfg(feature = "x")] pub mod` no longer
  loses `x`.
- **internal**: assert that every `.jinja` file on disk is present in its backend's template registration array.
  Template lookup resolves against a static array rather than the filesystem, so an unregistered template compiled
  fine and panicked at runtime; this happened three times in one day across two backends. The guard's first run
  surfaced 49 orphaned or superseded template files, all removed.
- **tests**: pin the working directory of `dart` and `kotlinc` child processes. Other tests mutate the
  process-global cwd into tempdirs that are then dropped, so an inherited cwd could already be deleted and the
  toolchain died at startup rather than reporting any result.


- **build observability**: emit centralized backend completion events with `duration_ms` and explicit success, failure,
  or skip outcomes for every configured language.

- **generation pipeline**: refresh cbindgen headers after generated FFI sources and backend post-build steps in both
  `generate` and `all`, then enforce source/header symbol parity before either command succeeds.

- **generated documentation**: remove poly's internal `~keep` token while preserving the surrounding public prose
  across every binding backend.
- **generated manifests**: replace hash-stamped Alef-owned TOML manifests with the current generated definition so
  stale dependencies and feature declarations cannot survive regeneration; continue refusing unmarked manifests.
- **FFI linting**: preallocate generated handle-request vectors so single-handle entrypoints pass crate-denied Clippy lints.

- **Zig snippets**: compile against the generated package's exported module so transitive imports declared by its
  `build.zig` remain available during validation.
- **build pipeline**: execute Gradle backends directly and make unsupported backend tools fail instead of reporting a
  successful no-op.
- **WASM snippets**: resolve local packages from wasm-pack's flat `pkg` output instead of a nonexistent `pkg/nodejs`
  subdirectory.
- **Go snippets**: construct generated DTO fields with the same optional/default pointer policy as the Go binding
  backend, preventing both pointer-to-value and value-to-pointer struct literal mismatches.
- Generate owned scalar handles for optional trait-bridge alias fields by cloning the configured handle, and reject
  non-Copy, non-Clone named field getters instead of silently returning null.
- Fix WASM manifests to enable configured binding-side feature gates by default, keeping exported factories available in ordinary `wasm-pack` builds.

- **Python snippets**: remove request types supplied by per-call native `from_json` overrides from public imports so
  the native class is imported exactly once and cannot shadow or be shadowed by the public type.
- **C snippets**: define standalone success guards and only rewrite the expected-result assertion, preventing error
  snippets from testing a result before its declaration or comparing scalar handles with pointer sentinels.
- **Java documentation**: escape Rust `\\u{...}` syntax before Java's early Unicode processing so generated Javadoc
  remains compilable.
- **Java trait adapters**: omit lifecycle overrides when a bridge has no configured super-trait, keeping generated
  adapters consistent with their managed interfaces.
- **Default constructor extraction**: preserve manually implemented `Default::default` as a generated static constructor
  across FFI, Python, PHP, R, and other method-based binding surfaces.
- **FFI default constructors**: retain canonical zero-argument `default` exports for lifetime-bearing owned values
  when conservative reference metadata is present, while continuing to exclude other borrowed returns and parameters.
- **Java visitor handles**: use the imported `List` type in generated cleanup tracking so strict Java lint does not
  report an unused import.
- **full generation convergence**: generate documentation snippets before rendering READMEs, so a clean `alef all`
  consumes the current run's snippets instead of requiring a second pass to add result-display statements.
- **Node declarations**: escape block-comment closers in Rust documentation before embedding it in generated
  TypeScript declaration comments.
- **snippets check**: run the configured audit and gap checks that `--help` already promised, scoped and gated to
  agree with `alef validate`'s existing snippet gate. Audit and gap checks see `docs.snippets.dirs` only —
  `inline_dirs` are prose pages whose fences are validated as snippets, never `--8<--` include targets — and a
  snippet counts as referenced when a `[crates.readme]` mapping, a generated-snippet coverage ledger, or a queried
  Astro content collection names it. Audit is skipped without a configured `docs_dirs`, and gaps are skipped
  without either `docs_dirs` or `required_languages`. Audit errors and structural gaps (missing include targets,
  missing required language variants, undocumented skips, unknown fence languages) fail the gate; unreferenced
  snippets remain a `strict`-only failure. A coverage manifest recording missing fixture/language cells stays a
  warning unless `strict`, as before, instead of failing reference resolution outright.
  **Newly fails:** an unparsable `docs.snippets.required_languages` entry is now an error rather than being
  silently dropped, matching `alef validate`.
- **Java errors**: align last-error dispatch with the shared FFI conversion, core, and panic taxonomy, and safely
  handle missing error context.
- **generate manifests**: reconcile Alef-owned generated TOML manifests before post-build processing, so newly
  generated binding dependencies are available without requiring a prior `alef scaffold` or `alef all` run while
  handwritten manifests remain untouched, and fail generation instead of continuing to dependent post-builds when
  the required scaffold manifest set cannot be produced.
- **C# service bindings**: invoke configurators through the native ABI, marshal named record parameters through
  owned scalar handles for configurators, registrations, and entrypoints, and propagate native conversion failures.
- **Java owned handles**: keep service and opaque owners closeable when transfer setup fails, contain handler upcall
  failures at the native boundary, and use the exact C ABI carriers for service metadata.
- **C# owned handles**: keep service and opaque owners closeable when transfer setup fails, lease service owners
  through registration calls, and defer trait-bridge cleanup until native release and active callbacks complete.
- **Go bytes**: pass the output pointer, length, and capacity required by every direct owned-byte return instead of
  treating an infallible byte function's integer status as a NUL-terminated buffer.
- **Go handles**: compare named parameter and return handles with the scalar zero sentinel in direct wrappers.
- **FFI scaffold**: declare `serde` directly in generated FFI crates now that the handle registry requires
  `serde::Serialize`, using the centralized template version and cargo-machete metadata rather than relying on the
  core crate's transitive dependencies.
- **FFI borrowed defaults**: restore free default-constructor exports that return owned lifetime-bearing values,
  storing them as serialized handles while continuing to exclude borrowed returns and borrowed-handle parameters.
- **documentation snippets**: read client credentials from each fixture's configured environment variable, falling
  back to the generic `API_KEY` name instead of publishing mock credentials in C, C#, Dart, Java, Kotlin, Python,
  Rust, and Swift examples.
- **WASM documentation snippets**: derive direct-call fixture eligibility from the target's exported function
  surface, recording unavailable imports as missing coverage while retaining client-wrapper recipes whose methods
  are reached through the resolved per-call or default factory.
- **java default values**: suffix integer defaults for boxed `Long` record components with `L`, so generated compact
  constructors compile while continuing to distinguish an absent value from an explicitly supplied zero.
- **FFI borrowed contexts**: restore owned lifecycle, field-accessor, owned-self method, and default-constructor
  exports for lifetime-bearing visitor contexts while continuing to reject APIs that pass borrowed handles across
  the ABI boundary. Non-`Send` contexts are stored as type-keyed serialized snapshots, preserving the registry's
  `Send` invariant and scalar handle ABI without erasing live visitor-context symbols.
- **e2e/ruby snippets**: bind collected streaming values through the configured result variable, keeping the
  assignment and subsequent `puts ...inspect` reference synchronized instead of binding an unused `chunks` variable.
- **e2e/elixir snippets**: bind collected streaming values through the configured result variable, keeping the
  assignment and subsequent `IO.inspect` reference synchronized instead of binding an unused `chunks` variable.
- **R default arguments**: only call a generated class's `$default()` wrapper when that class is actually eligible for
  extendr registration, preventing required options from referencing removed wrappers.
- **Go snippet validation**: preserve configured `GOMODCACHE` and `GOPATH` paths in the sanitized tool environment
  and derive Go's home-based defaults when they are not explicitly exported, so generated snippets can reuse
  available modules instead of failing before validation.
- **C documentation snippets**: keep configured client-method identities separate from prefixed ABI symbols, so
  adapter metadata resolves and real client/streaming examples are emitted; unresolved recipes now enter the
  missing-coverage ledger instead of compiling as successful diagnostic skip stubs.
- **readme tests**: align the structured function-surface template fixture with Minijinja's Boolean rendering.
- **zig visitor tests**: assert scalar-handle serialization through the configured FFI symbol instead of a
  hardcoded placeholder name.
- **e2e/zig visitors**: treat generated FFI result handles as scalar integers, using the zero sentinel instead of
  optional-pointer comparisons, captures, and unwraps while preserving pointer handling for returned JSON strings.
- **verify**: reject Alef-owned generated files whose header remains but whose `alef:hash` stamp is missing, so a
  mixed stamped/unstamped generated tree cannot pass freshness verification.
- **Python snippets**: bind successful non-void call results even when `docs.shows` or a presentation recipe consumes
  them, so generated examples display useful values instead of discarding the call result.
- **documentation snippets**: display successful non-void Rust, Swift, Zig, R, and Kotlin call results in generated
  examples; PHP already consumed these values.
- **FFI error header**: keep `AlefFfiErrorCode` reachable through generated cbindgen export filters and avoid repeated
  `ErrorError` tokens where an error type and variant meet in public C enum members.
- **FFI error enum members**: collapse consecutive repeated words inside the error type path, so a crate laying its
  error type out as `my_crate::error::Error` emits `MyCrateErrorNotFound` rather than `MyCrateErrorErrorNotFound`.
  The previous pass only elided the repeat at the type/variant boundary and left the path-internal stutter intact.
- **FFI error enum members**: namespace alef's five built-in codes with the project ABI prefix, so `None` is emitted as
  e.g. `SampleAlefNone`. cbindgen applies `[export] prefix` to the enum type but copies member names into the header
  verbatim, and C enum members are global identifiers — the bare names collided with platform headers (X11 defines
  `None` as `0L`) and with any second alef-generated library in the same translation unit.
- **kotlin errors**: let unnumbered error variants use the runtime fallback instead of panicking while generating
  Kotlin/Native bindings that mix explicitly numbered and fallback variants.
- **Ruby/Magnus errors**: reconstruct tuple error variants with positional Rust syntax for every binding
  representation that emits tuple variants, including adjacently tagged enums, while retaining struct syntax for
  named variants and bare syntax for unit variants.
- **generated-file provenance**: align hash extraction with the raw header window used by injection, including a stamp
  emitted at zero-based line 10 after Markdown frontmatter, and only strip exact generated stamp shapes immediately
  following an Alef header marker so hash-like body prose remains untouched.
- **zig errors**: let unnumbered error variants use the stable unknown-code fallback instead of panicking while
  generating bindings that mix explicitly numbered and fallback variants.
- **test fixtures**: keep the version-pin fixture aligned with the root-flat config shape and give the Go capsule
  fixture the complete borrowed-static ABI contract required by capsule validation.
- **dart/flutter_rust_bridge**: give FVM a persistent Alef cache when running
  `flutter_rust_bridge_codegen`, so clean regeneration worktrees reuse the installed Flutter SDK instead of
  downloading it again. Explicit `FVM_CACHE_PATH` and legacy `FVM_HOME` settings remain authoritative.
- **dart/flutter_rust_bridge**: reuse a persistent, crate-scoped Cargo target directory for FRB macro expansion.
  Clean regeneration worktrees now retain Cargo fingerprints, dependencies, proc macros, and build-script artifacts
  instead of recompiling the full Rust crate for every `cargo expand`; an explicit `CARGO_TARGET_DIR` still wins.
- **wasm scaffold**: publish a conditional `exports` map that resolves package self-imports to generated Node entrypoints
  while keeping the browser condition on the explicitly initialized web build. This lets generated snippets and e2e
  tests import the package by name after `wasm-pack` builds its target directories.
- **FFI errors**: replace unstable hash-derived domain error codes with explicit `#[alef(error_code = N)]` allocations,
  validate their public range and uniqueness, and emit a cbindgen-visible `AlefFfiErrorCode` enum. Unannotated variants
  now use the stable `Unknown = 2` fallback instead of accidentally creating a rename-sensitive ABI contract.
- **e2e/dart snippets**: stop emitting a call to the undefined `_fixtureUrl` helper in doc snippets for
  `client_factory` calls. The helper is defined only by the full e2e test-file emitter, never by the standalone
  snippet emitter, so every snippet constructing a client failed to compile with "The function '_fixtureUrl' isn't
  defined." Snippets now build the client with just the API key, matching the PHP, Ruby, Go, and TypeScript emitters,
  which likewise omit the mock-server `baseUrl` from their doc snippets.
- **Python snippets**: import every symbol the emitted snippet body references. Import candidates were
  computed only when a fixture group held at least one fixture *not* skipped for Python, but a docs
  snippet is emitted for every fixture regardless of skip status and lifts its import block out of that
  same rendered test file. A Python-skipped fixture therefore produced a snippet whose body called the
  configured client factory and constructed the request type while importing neither, and snippet
  validation failed with `unknown-name`. Candidates are now derived for every non-HTTP fixture and then
  pruned to the identifiers the emitted unit actually references, so nothing referenced goes unimported
  and nothing imported goes unreferenced.
- **C snippets**: define the `ALEF_TEST_SKIP` guard macro inside every emitted snippet that references it.
  A fixture declaring `[env] api_key_var` without a mock server renders an `ALEF_TEST_SKIP(...)` env guard,
  but the macro is declared only in the generated e2e runner header, which a standalone documentation
  snippet never includes — so the emitted translation unit failed to compile with a
  `call to undeclared function 'ALEF_TEST_SKIP'` error. The snippet-local definition returns `EXIT_SUCCESS`
  from `main` rather than the runner's bare `return`, which is valid only inside its `void test_*(void)`
  functions, and is `#ifndef`-guarded so an enclosing definition still wins.
- **csharp**: derive the element type of an array-valued field in an object initializer from the owning
  struct's `Vec<T>` field rather than hardcoding `List<string>`. Genuinely typed collections
  (`List<Message>`, `List<RerankDocument>`) were emitted as string lists, which does not satisfy the
  generated property, and scalar fields binding to `JsonElement` were emitted as bare literals. Both now
  route through the same per-element `JsonSerializer.Deserialize<T>` rendering that top-level array args
  already use, falling back to `List<string>` only when the element type is genuinely unresolvable.
- **wasm/typescript**: emit the raw value for a `#[serde(untagged)]` data enum field instead of an enum
  member reference. Such an enum serializes as the bare payload of whichever variant matched, so a
  string-typed instance is the JS value itself; treating it as `EnumType.Variant` turned an empty string
  into `WasmEmbeddingInput.`, a syntax error. Mirrors the representation gate the napi `.d.ts` dispatcher
  uses.
- **napi**: declare internally-tagged enums with newtype-of-struct variants as the flat optional-field
  object the napi glue actually emits, instead of a discriminated union keyed by the tuple field's
  synthetic `_0` name. The generated `.d.ts` previously leaked the internal field name as a literal `0:`
  property (e.g. `{ role: 'system'; 0: SystemMessage }`) and wrapped each variant as its own union member,
  when the compiled `#[napi(object)]` struct is actually one type with every variant's field present as an
  optional property (e.g. `{ role: 'system' | 'user'; system?: SystemMessage; user?: UserMessage }`).
- **napi**: dispatch the `.d.ts` enum arm on the enum's actual serde representation rather than on
  whether any variant carries a payload. `is_data_enum` gated on `serde_tag.is_some()` *and* at least
  one variant having fields, and was the arm's only switch, so two representations fell through to the
  plain string-enum branch and silently lost their wire shape. An internally-tagged enum whose variants
  are all unit variants serializes as `{"kind":"A"}`, not the bare string `"A"`; the glue generator
  gated on the same condition, so the binding conversion was wrong in the same way and both are
  corrected together. An untagged (`#[serde(untagged)]`) data-bearing enum serializes as a bare union
  of each variant's own shape and was likewise declared as a payload-free string enum, discarding every
  field. Externally tagged data enums remain unhandled and are tracked separately — declaring a shape
  the runtime does not produce would be worse than the current omission.
- **snippet typecheck validation**: stop reporting `effective_level: typecheck` for PHP, Ruby, and Elixir
  snippets that were never actually type-checked. Every level below `Run` ran the same syntax-only check
  (`php -l`, `ruby -c`, `Code.string_to_quoted`) while the validator declared `max_level: Run`, so a
  snippet referencing an undefined class, constant, or module passed as a `typecheck` result — the entire
  population of typecheck passes for these three languages was unverified. A validator can now report,
  through a new `achievable_level` hook, the deepest level its *current environment* actually backs,
  separately from `max_level`'s fixed per-language ceiling; a `typecheck` request for these three
  languages now reports `effective_level: syntax` and counts as `downgraded`, which a strict run treats
  as incomplete coverage instead of a pass. No zero-config real type-checker exists for any of the three
  that can safely analyze an isolated snippet: PHP's PHPStan/Psalm need the project's composer autoload
  or every legitimately external symbol reads as unresolvable, Ruby's Sorbet/RBS need project-wide
  `# typed:` sigils the generated snippets don't carry, and Elixir's Dialyzer needs an out-of-band PLT
  this harness doesn't build. Wiring one in with the project context it needs is left for a follow-up.
- **snippet typecheck validation**: apply the same `achievable_level` cap to Bash and R snippets. Both ran a
  syntax-only check below `Run` (`bash -n`, R's `parse(file = ...)`) while declaring `max_level: Run`, so a
  snippet referencing an undefined command or function likewise passed as an unverified `typecheck` result. A
  `typecheck` request for either language now reports `effective_level: syntax` and counts as `downgraded`.
  ShellCheck and R's `codetools::checkUsage`/lintr exist but aren't wired up here, so `typecheck` must not be
  claimed until they are.


- **C# e2e**: omit the illegal `private` modifier from file-scope generated visitor classes while preserving the
  same class shape when the visitor appears inside a nested test container.
- **go**: compare value receivers returned by `_from_json` against the scalar `AlefHandle` zero sentinel instead of
  `nil`, allowing generated non-opaque methods to compile under cgo.
- **Zig snippets**: import the configured binding module rather than the e2e release-package alias, keeping registry
  artifact naming separate from the public module identifier.
- **Dart snippets**: import the configured public library entrypoint instead of deriving the filename from the Rust
  crate name, while preserving the separately configured e2e dependency alias.
- **snippets**: let a fenced code block's own tag win over the file's front-matter `language:` during discovery. A
  markdown file with multiple fenced blocks in different languages (e.g. a `toml` config block followed by a `json`
  block) had every block forced onto the front-matter language, so the non-matching blocks were validated with the
  wrong toolchain and failed for reasons unrelated to their actual content. An unrecognized or absent fence tag still
  falls back to the front-matter language, then to the directory-derived language.
- **CLI**: `alef all` now synchronizes registry package versions before generation and reloads changed configuration,
  allowing stale Zig registry hash version prefixes to self-heal instead of aborting clean canaries.
- **napi**: omit `..Default::default()` from fully populated adjacent-enum constructors while retaining it for
  partial variants that still require defaulted fields.
- **rust snippets**: locate generated test-function boundaries without treating braces inside raw strings, escaped
  strings, or comments as the function terminator.
- **rust snippets**: declare the `tokio` dependency an async snippet needs. `rust/snippet_body.rs.jinja` emits
  `#[tokio::main]` for every async fixture, but the snippet carried no matching crate requirement, so the validator
  built a check project with nothing in `[dependencies]` and every async Rust snippet failed on E0433
  (`use of unresolved module or unlinked crate tokio`) and E0752 before any of the behaviour it demonstrates was
  checked. The requirement is pinned with `features = ["full"]`: `#[tokio::main]` lives behind tokio's `macros` and
  `rt-multi-thread` features, so a bare version line resolves the crate and still fails to compile.
- **zig snippets**: pass the include directories the build manifest declares to the reconstructed `build-exe`
  command. The session validator read the manifest for its module name and root source only and discarded the
  `addIncludePath` the same file declares, so a snippet reaching a `@cInclude` inside the binding failed with
  `C import failed ... 'header.h' not found` while `zig build` against that identical manifest succeeded — the
  harness was validating something other than what ships. An include path bound through a
  `b.option(...) orelse "<default>"`, the shape Alef's own `build_zig.jinja` emits, resolves to its default; any
  other expression is skipped rather than guessed at, since a wrong `-I` is worse than none.
- **e2e snippets**: use the configured exported error type instead of fabricating one from the crate name, and emit
  Go error values with the correct non-pointer shape.
- **scaffold**: derive Python, PHP, and FFI `.gitattributes` entries from the source crate directory, matching the
  binding crate paths Alef actually scaffolds when the configured package name differs from its Rust crate path.
- **zig scaffold**: emit the example with Zig 0.16's `std.Io` API and avoid an unused allocator binding, so the
  scaffolded example compiles with the supported Zig toolchain.
- **e2e/node**: import enum classes referenced as runtime values by generated typed-input builders, including enum
  fields discovered recursively from the IR rather than declared in per-language overrides.
- **validate**: stop treating vendored/fetched copies of a crate's `Cargo.toml` as authoritative when checking
  `Cargo.lock` versions. `alef validate versions` built a single name-to-version map from every `Cargo.toml` under
  the workspace root, keyed by package name alone. A frozen manifest left behind by dependency vendoring — a
  Rustler `vendor/` tree carried for offline builds, or a Mix `deps/` fetch of a published package that bundles its
  own native crate source — declares the same package name at whatever version was current when it was
  vendored/fetched, silently overwriting the live crate's entry in the map. Every other `Cargo.lock` in the repo
  that already matched canonical then compared against the stale vendored version and was reported as a
  `[MISMATCH]`, even though the lockfile agreed with the live source. `vendor` and `deps` directories are now
  excluded from both the `Cargo.toml` manifest scan and the `Cargo.lock` check, the same way `target`/`.git` already
  are. Genuine drift in a real (non-vendored) `Cargo.lock` still fails.
- **cli**: stop `generate`, `scaffold`, and `all` from silently rewriting `alef.toml`'s `alef_version`. The pin never
  gates generation and projects may coordinate it with external installer or workflow pins that generation cannot
  update; release version sync remains explicit.
- **validate**: stop descending into nested git checkouts that `.gitmodules` does not register. `alef validate
  versions` walked every directory under the workspace root when collecting `Cargo.toml`/`Cargo.lock` files,
  including a linked `git worktree` checked out inside the repo (e.g. under `.worktrees/`) — an independent checkout
  sitting at a different commit. Its manifests reported `[MISMATCH]` noise against the host repo's canonical
  version, could poison the version map for a package name it happens to share with the live tree, and — for a
  worktree mid-regeneration — could differ between two runs of the same command. A directory whose root carries a
  `.git` entry is now skipped, but **only** when `.gitmodules` does not register it as a submodule path: a
  registered submodule is a declared part of the repo's version surface and is still walked by both the manifest
  scan and the `Cargo.lock` check, so genuine drift inside a submodule keeps failing.
- **ffi**: return the scalar `0` sentinel from string-bridge parameter and UTF-8 guard failures when the exported
  ABI returns `AlefHandle`, instead of emitting `null_mut()` and producing uncompilable generated Rust.
- **zig snippets**: omit the parsed-result binding when generated assertions never reference it, avoiding an
  unused-local compile error while preserving the binding for assertions that consume the result.
- **java scaffold**: exclude Alef's `.alef` validation scratch directory from Maven Checkstyle scans without
  suppressing violations in user source files.
- **go**: represent generated FFI handles consistently as scalar `AlefHandle` values, including visitor and
  options conversion paths, and compare their failure sentinel against `0` instead of `nil`.
- **C examples and e2e**: render scalar handle declarations and absent-value sentinels as `AlefHandle`/`0` while
  retaining `NULL` for pointer-valued strings and other pointer parameters.
- **ffi**: honor configured extra Clippy allowances and allow `collapsible_if` in generated crates without raising
  their minimum supported Rust version.
- **e2e/ruby**: emit a fixture category's spec file whenever its fixtures render executable examples. The
  category-level gate in `ruby.rs` decided whether to emit the file using a predicate that omitted `is_streaming`,
  while the per-fixture branch in `spec_file.rs` decided what to put in it using one that included it. A category
  whose fixtures were all streaming was therefore dropped whole — no file — even though the ruby streaming emitter
  renders those fixtures fully. Nothing downstream notices an absent category: `alef verify` walks emitted markers,
  the empty-category check in `e2e/validate.rs` only fires when *every* configured language skips a category, and
  `fixture_inclusion` never consults an emitter's capability. Both callers now share one predicate so they cannot
  drift apart again, and a category that genuinely renders nothing executable is logged instead of vanishing.
- **generate**: run the converging formatting pass on `alef all`, so a full regen lands committable. `alef all`
  called `format_generated` with `Some(&changed_languages)`, which selects the single-pass branch; the convergence
  loop — written because poly's `.cs`/`.java`/`.json` engines are not single-pass idempotent, and documented as
  serving "the `alef all` path" — was therefore unreachable from the one command that regenerates everything. A
  regen left formatting drift that `poly fmt --check` rejected, `finalize_hashes` then stamped provenance over that
  drift, and a second `alef all` silently settled it — which is why regenerating twice produced changes the first
  run should have made. The language filter was also wrong for the workspace-wide `cargo sort -n -w` folded into
  that loop, which must cover crates the current run did not generate.
- **defaults**: carry the elements of a non-empty collection literal through the IR instead of discarding them.
  Every `vec!`/`hashmap!`/`hashset!` default collapsed to `DefaultValue::Empty`, so a Rust default of
  `vec!["noscript"]` reached the backends indistinguishable from `vec![]` and every binding emitted an empty
  collection — a silent cross-language behavioural divergence, not a cosmetic one. The guard that appeared to
  separate the two cases was dead code: both of its branches returned `Empty`. A new `DefaultValue::ListLiteral`
  carries the elements, and Rust, Python, Kotlin, Swift, Dart, C#, Elixir and the docs renderer emit them. Go and R
  deliberately keep falling back to the empty collection, because Go needs the element type spelled out
  (`[]string{…}`) and R's `c()` carries vector-coercion semantics of its own — guessing either risks a default that
  differs from the Rust one. A genuinely empty `vec![]` still lowers to `Empty`, and a literal containing any element
  that cannot be rendered self-containedly — notably a function call — falls back to `Empty` whole rather than
  lowering a partial list. **Consumer-visible:** a field whose Rust default is a populated collection now generates
  that collection as its binding default instead of an empty one.
- **snippets**: scope the shared validation timeout to validators that genuinely batch. `run_validation` handed a
  single wall-clock budget, keyed by language/session/level, to the per-snippet path — which is reached only by
  validators that spawn one toolchain process per snippet (every language except Rust below `Run`). The first
  snippet in a group could consume the whole budget, after which the runner short-circuited every remaining snippet
  with a synthetic `Timeout` error naming a `<language> validation batch` command that was never spawned. Docs runs
  therefore reported timeouts that varied with snippet ordering and machine speed rather than with the snippets.
  Each snippet on that path now receives the configured `timeout_secs`; group budgeting is retained in
  `validate_batches`, where one process really does cover N snippets. **Consumer-visible:** a language group's
  worst-case wall clock is now `snippets × timeout_secs` rather than `timeout_secs`, so a `timeout_secs` tuned
  against the old shared budget may need lowering.
- **capsules**: expose `shares_native_runtime` for Kotlin Android and enforce capsule ownership/ABI contracts during
  Go, Swift, Zig, and Kotlin Android generation instead of leaving those backends outside the validation gate.
- **e2e/zig**: emit streaming e2e tests instead of discarding them, and never drop a fixture category in silence. A
  hardcoded zig-only filter excluded every fixture whose resolved call declared `streaming = true`, although the zig
  streaming emitter is fully written; the exclusion is now narrowed to the case it was guarding — a streaming call
  with no `client_factory`, which zig cannot render because streaming is exposed only as a method on the handle. A
  category left empty by that filter previously hit a bare `continue`: no file, no log and no gate failure, because
  `alef verify`, the empty-category check in `e2e/validate.rs` and `fixture_inclusion` all still reported zig as
  included, so a consumer whose config routed a streaming call to zig received nothing and was never told. Such a
  category now emits a placeholder suite naming every dropped fixture, plus a warning, so an unemittable category is
  visible in the output tree instead of vanishing.
- **java**: box a config field whose serde default is a non-zero integer literal, so a caller's explicit `0` is no
  longer overwritten by the default. Java records carry no per-component defaults, so a default is restored in the
  compact constructor by testing the incoming value, which requires a value meaning "nothing was supplied".
  `Duration` and `#[serde(default)]` fields already boxed for that reason; a plain literal default did not, leaving
  `0` as the only available sentinel — `if (maxRedirects == 0) { maxRedirects = 10; }` silently gave 10 redirects to
  a caller who asked for none. The "must box" predicate now covers a non-zero literal default and is applied at
  every site that has to agree — record component type, `@Nullable`, the compact constructor's condition, and both
  the builder field and setter types — through one shared helper, because a disagreement between them emits a record
  and a builder declaring different types for the same field. Java-local by construction: Python, C# and Kotlin emit
  per-field initializers and need no sentinel. **Consumer-visible:** affected record components and builder setters
  change from the primitive type to its boxed form.
- Make generated parameter-conversion failures use the scalar zero sentinel whenever the exported FFI return type is
  `AlefHandle`, even when the source return metadata has a pointer-shaped fallback.
- **config generation**: report every Rust-binding field whose serde default function cannot be preserved in one
  actionable generation error instead of panicking on the first field. The diagnostic now identifies each owning
  type, field, and function and explains the public, unconditional static-method or literal-default remedies.
- **validate**: compare csproj `AssemblyVersion`/`FileVersion` in the 4-component .NET form rather than against raw
  canonical SemVer. The generator stamps both fields through `to_dotnet_assembly_version` — `1.17.0` becomes
  `1.17.0.0`, and a prerelease such as `1.9.0-rc.48` becomes `1.9.0.0` — because .NET rejects SemVer prereleases in
  those attributes. `alef validate versions` compared all four csproj fields against the raw canonical string, so the
  two assembly fields could never match and every consumer with C# enabled reported permanent mismatches on output
  alef itself is required to produce. Under `--exit-code` that is a permanently red release gate. `Version` and
  `InformationalVersion` carry the full SemVer and keep comparing raw, so the four-component form is still rejected
  there.

- **csharp**: only emit the `Register<Trait>` facade when the trait bridge declares a `register_fn`. The facade calls
  `NativeMethods.Register<Trait>`, which is generated solely for bridges that have a native register function, so a
  bridge configured without one produced a call to an undeclared member — `CS0117`, failing the whole package build.
  The unregister facade two lines below was already gated on `unregister_fn`, and Java (`gen_bindings/facade.rs`) and
  Go (`gen_bindings/mod.rs`) already gate on `register_fn`; C# was the sole outlier. Emission is narrowed rather than
  a declaration invented: no register-shaped symbol exists in the Rust exports, the cbindgen header or the built
  dylib, so declaring the extern would convert a build error into a runtime `EntryPointNotFoundException`.
  **Consumer-visible:** bridges without a `register_fn` no longer expose a `Register<Trait>` method that could never
  have worked; bridges with one are unaffected.
- **e2e/fixture**: accept a `skip.languages` id naming a known e2e target that isn't configured in this run, so a
  consumer holding a backend out of `[languages]` keeps its skip entries valid. Typo'd ids still fail, and `"ffi"`
  stays rejected because `Language::Ffi` maps to the `"c"` generator.
- **ffi/e2e/c**: keep options-field bridge functions, options, results, and visitor callbacks on the scalar
  generational `AlefHandle` ABI used by every ordinary managed value. The special options-field bridge bypass still
  emitted raw pointers after the 0.61 handle migration, while its JSON and visitor constructors returned scalar
  handles; generated C tests and snippets therefore failed strict compilation in both pointer-to-integer directions.
- **docs/snippets**: make a strict-mode failure actionable. `strict snippet validation failed for crate X: N
  validation(s) downgraded` reported only a total — no language, no snippet id, and no level transition — and the
  achieved level is not recorded in the emitted snippet frontmatter, so there was no other way to learn which
  snippets regressed or from what level. A real run reported 261 downgrades with no entry point to any of them.
  Strict failures (downgraded, unavailable, and failed/errored) now append a per-language tally with up to three
  sample ids and their `requested -> effective` transition, and say how many were elided. The validation report
  configured by `report_output` is also written *before* the strict bails rather than after: a run that fails
  strict mode is precisely the run whose report is needed, and emitting it afterwards meant the artifact was never
  produced in that case.
- **e2e/format**: make generated output reproducible when a formatter fails. Languages were collected into a
  `HashSet` and formatted in its iteration order, which is randomly seeded per instance, and the loop aborted on
  the first failure with `?`. Together those meant one failing language left a *different, arbitrary* subset of
  the remaining languages unformatted on every run: regenerating an unchanged tree produced different bytes.
  Observed on a consumer whose registry-mode Go test app cannot pass `go mod tidy` before its version is
  published — two consecutive `alef all --clean` runs differed in 143 files, the next pair in 47, all under
  `test_apps/`, with clang-format applied to a varying subset. Languages are now formatted in sorted order and
  every language is attempted before failures are reported, so the emitted tree no longer depends on ordering or
  on whether an earlier language failed. The run still fails, and now names every language that failed rather
  than only the first.
- **tests**: refresh Kotlin Android, Swift, and Zig snapshots so the checked-in expectations cover the integrated
  JNI path discovery, serde bridge grouping, numeric error dispatch, and fallible string ownership behavior.
- **ffi/scaffold**: declare every feature named by a `#[cfg(feature = "X")]` gate the codegen emits into the
  generated FFI crate. Cargo features are per-crate, so a wrapper whose `[features]` table forwards only
  `full = ["<core>/full"]` never defines `X` itself, even when `full` enables `X` on the core dependency. The
  emitted gate was therefore unsatisfiable under *every* feature selection: `rustc` reported it as an
  `unexpected cfg condition value` warning and silently dropped the item, while `cbindgen` — which does not
  evaluate the gate — kept declaring it in the header. The crate still built with exit 0, so the failure surfaced
  only downstream, as a link error or `dlsym` miss for every C-ABI consumer against a header that promises symbols
  the `cdylib` does not export. Features discovered from emitted gates are now declared as passthroughs and default
  ON, preserving the surface each gate was written to expose while keeping it switchable; `[crates.ffi]
  extra_features` keeps its documented declare-but-do-not-enable behaviour for mutually-exclusive alternatives such
  as a `wasm-http` backend, and a gate naming one of those entries does not promote it into `default`. This mirrors
  the feature collection the dart, wasm, swift, and extendr backends already perform.
- **e2e**: stop registry-mode dependency resolution from making a release unreachable. Registry-mode test apps pin
  the version the current run produces, so any post-generation step that resolves those manifests against a registry
  (`go mod tidy`, or a user `format` override shelling out to a resolver such as `bundle exec`) cannot succeed until
  that version is published — while publishing requires the run to finish first. The failure aborted
  `run_formatters` mid-stage, so `finalize_hashes` for the test apps, the orphan sweep, the stage cache write and the
  entire **docs** stage never ran: one unpublished package left the test apps unstamped and every generated docs page
  stale, and re-running could not converge. Formatting itself is unchanged and still aborts generation in every mode
  — poly and `mix format` need no registry, so they have no pre-release excuse. Only dependency resolution is
  deferred, and only under `DependencyMode::Registry`: `go mod tidy` is skipped and recorded, and a failing user
  `format` override is recorded rather than fatal. `run_formatters` now returns the deferred steps as
  `Vec<DeferredFormatting>`, which `alef all` reports once the pipeline has completed. `DependencyMode::Local` — the
  mode that actually gates correctness — is behaviourally identical and always yields an empty list.
  **API-visible:** `run_formatters` and `run_formatters_for_cached_paths` return `Vec<DeferredFormatting>` instead of
  `()`.
- **kotlin-android**: emit a handle wrapper class only for opaque types some visible top-level function returns.
  A type that is `is_opaque` but that nothing returns cannot be constructed from Kotlin at all, yet still got a
  `<TypeName>.kt` whose `close()` called `nativeFree<TypeName>` — a symbol the Bridge object never declares and the
  native JNI shim never implements, so the generated module failed to compile with `Unresolved reference` on a class
  no caller could have instantiated. Matches the reachability predicate the JNI shim generator and the Bridge
  destructor emitter already apply.
- **jni/scaffold**: inherit the core crate's configured feature set when Kotlin Android does not provide a
  backend-specific override, while preserving explicit per-backend features.
- **e2e/wasm (tests)**: cover the fully-excluded-category path through `WasmCodegen::generate`, not only through the
  renderer. The existing unit test called `render_wasm_excluded_category` directly, so it verified the renderer but
  never that `generate` invoked it — reintroducing the silent-drop regression left all seven wasm unit tests green.
  The new integration test drives `generate` end to end and fails when the category is dropped.
- **generate**: record scaffold-emitted manifest paths (`composer.json`, `package.json`) in a durable
  per-crate manifest and reclaim them, along with their package-manager lockfile siblings, when a later
  `alef all` run stops emitting them at that path. Every existing orphan-cleanup input (`write_lang_manifest`,
  the `generate-{lang}-ownership` stage) filters scaffold paths through `carries_alef_marker()`, which a
  `generated_header: false` manifest never satisfies, so `sweep_manifest_orphans` was always called with an
  empty `previous_paths` for these files and could never reach manifests left behind by a layout change, even under
  `--clean`. The marker-free route is confined to an explicit two-name allowlist of
  manifests that are structurally incapable of carrying a marker; a lockfile is never reclaimed on its own
  provenance, only as a cascade of its manifest being reclaimed.
- **generate**: refuse to write a `generated_header: true` scaffold/e2e file over pre-existing content that
  carries no `alef:hash:` marker, instead of stamping or overwriting it unconditionally. A plain `alef all
  --clean` run could silently claim and stamp hand-written files because the write path only ever asked "does this
  run want to emit here", never "has alef ever recorded owning this exact path". The check is marker-based
  rather than cache-based (`.alef/`'s manifests are gitignored local scratch space that does not survive a
  fresh clone or a cache-less CI job), so it holds durably across sessions without narrowing any legitimate
  regeneration. The check is skipped for paths alef cannot stamp (`.md` above all — no generated README has
  ever carried a marker), where a missing marker is not evidence of foreign content and enforcing would
  freeze regeneration permanently.
- **e2e/elixir**: emit `test/test_helper.exs` with `generated_header: true` instead of `false`. As a
  `generated_header: false` seed it never carried a marker even though alef legitimately re-authors it on
  every run, so the write path had no durable way to tell "alef's own unmarked output" apart from
  "hand-written foreign content" for it.
- **e2e/kotlin**: honor `fields_json_scalar` for fixture field paths that carry a virtual namespace prefix (e.g.
  `interaction.action_results[0].data`), and accept the same bracket-wildcard/de-indexed spellings already
  interchangeable in `fields_optional`. `field_is_json_scalar` compared the raw fixture path directly against
  the configured set, but `accessor()` strips the namespace prefix before building the field expression — so a
  `fields_json_scalar` entry configured against the stripped struct path (`action_results[].data`) never
  matched, and the field fell through to the plain `.orEmpty()` fallback. `.orEmpty()` is a `String?`
  extension undefined on the `Any?` a JSON-scalar field actually has, leaving the generated Kotlin e2e module
  uncompilable.
- **magnus**: apply a field's real `#[serde(default = "...")]` value instead of raising `missing required
  field` for it. Any `Named`-typed field without an `EnumVariant` typed default was treated as required,
  which also caught fields that carry a genuine callable default (`FunctionCall`/`PublicFunctionCall`) —
  silently dropping the default and forcing Ruby callers to pass the field explicitly, even though the
  generated `.rbs` stub still declared it optional. The default's return value is converted with `.into()`
  for the same reason as the wasm fix: Magnus mirrors `Named` types into their own `#[magnus::wrap]` struct,
  a distinct Rust type from the core one under the same short name.
- **wasm**: convert a `#[serde(default = "...")]` function's return value into the field's wrapper type in
  generated constructors. A defaulted field whose type is `Named` (e.g. `ssrf: SsrfPolicy`) is mapped to a
  distinct binding wrapper (`WasmSsrfPolicy`), but the constructor's fallback expression called the core
  default function directly without `.into()` — an `E0308` type
  mismatch that broke every wasm build with a defaulted, wrapped-type field.
- **e2e/wasm**: stop silently dropping a fixture category when every fixture in it is excluded for wasm (e.g. an
  entire `visitor` category skipped via `skip.languages`); emit a placeholder suite naming each excluded fixture and
  its reason, and log a warning, instead of generating no output at all.
- **e2e/r, e2e/c**: stop trimming only the actual side of an `equals` assertion. R wrapped the result in `trimws(...)`
  while emitting the fixture's expected literal verbatim, and C routed string equals through a helper that trimmed
  trailing whitespace off the actual value only; both made assertions against expected values with a legitimate
  trailing newline permanently unsatisfiable. `equals` now compares both sides exactly, matching every other
  generated language.


- **config/services**: accept registration variant `languages` maps and carry each canonical language's style,
  handler shape, and method prefix through extraction into service generation. Python decorator overrides now use
  their registered overload templates instead of failing after config parsing.
- **ffi/services**: carry an explicit response-deallocator callback beside every service handler callback and invoke
  it after copying the response but before fallible deserialization. Rust previously called process-global `free()`
  on host allocations, including C# `Marshal.StringToCoTaskMemUTF8` memory, which crosses allocator families and can
  corrupt the Windows heap. C#, Java, Go, and Zig service registrations now pass their matching deallocator, and the
  generated service wrappers use the scalar `AlefHandle` carrier consistently.
- **csharp/zig**: invalidate host wrappers immediately after a consuming native method returns, including error
  returns, so fluent builders cannot retain a stale owner while wrapping the replacement handle.
- **ffi/csharp/zig**: return every byte buffer through the owned pointer/length/capacity ABI, including infallible
  borrowed slices, so hosts copy exact binary data and free only the allocation transferred by the FFI layer.
- **java/service**: bind service constructors, owners, metadata, registration variants, and entrypoints with the
  scalar `AlefHandle` carriers and canonical C export names emitted by the FFI backend; marshal text metadata to
  native segments, invoke Panama handles with typed arguments, and validate the service symbols at load time.
- **extract/ffi**: recognize serde implementations that derive one direction and implement the other manually, so
  named parameters such as authorization configuration emit the FFI JSON constructor their Java and C# wrappers call.
- **config**: reject unknown keys in closed Alef configuration sections instead of silently discarding misspelled or
  misplaced settings; extension maps remain open where arbitrary names are part of the schema.
- **ffi/java**: always emit JSON constructors for serializable FFI types and declare matching Java lifecycle handles,
  including types reached through generated facade fields rather than direct function parameters.
- **go/visitor**: keep options-field visitor context and result types out of generic binding and method emission, so
  generated Go packages no longer call FFI symbols intentionally omitted for borrowed visitor-associated types.

- **kotlin-android**: locate the configured JNI crate by walking from the generated Gradle project, accept an
  explicit manifest override, and copy host libraries from the Cargo workspace target directory.

- **swift**: emit reachable opaque-handle type aliases even when no capsule mapping is configured, while avoiding
  duplicate declarations for client and capsule types.
- **e2e/dart**: emit trait-bridge stub factories in standalone snippets and avoid binding `Future<void>` calls to a
  result variable.
- **e2e/snippets**: prune previously generated snippet files that a later successful run no longer produces, using
  recorded generation ownership and language-scoped path differences without deleting hand-authored files.
- **pipeline/cache**: discover and re-stamp Alef-owned generated files omitted from a cache-hit language's in-memory
  path set, including files whose provenance hash line was previously stripped.
- **sync-versions**: keep C# assembly/file versions and the registry Rust test-app package version aligned with the
  consumer SemVer; .NET assembly fields now use the required four-component numeric form.
- **scaffold/poly**: lower configured exclusions separately for discovery gitignore semantics and hook glob matching,
  preventing nested fixture over-pruning and missed nested build/vendor directories.
- **docs/zig**: render function and enum-value identifiers in snake_case while retaining PascalCase type names in
  generated reference pages.
- **e2e**: use one normalized indexed-path convention across optional matching and eight accessor renderers, including
  TypeScript, Node, C#, Zig, Kotlin, Kotlin Android, Swift, and Dart.
- **e2e/go**: resolve fixture fields through their serde wire names so renamed DTO properties are populated instead
  of silently disappearing from generated Go literals.
- **e2e/typescript**: qualify inferred enum-field metadata by its owning type so an enum field cannot poison a
  same-named scalar field on another generated DTO.
- **snippets/strict**: treat a validator's declared `max_level` as a capability ceiling rather than a downgrade.
  A snippet that passes at its validator's maximum was reported as downgraded, and `strict` fails on any downgrade,
  so requesting a level that any validator caps below — `typecheck` with zig capping at `compile`, or toml/json/yaml
  capping at `syntax` — could never pass however healthy the environment was, and the only consumer workaround was
  lowering the level for every other language too. Such results now pass, carry a `capability_capped` flag, are
  counted in the run summary, and are surfaced through an explicit warning. Annotation-driven downgrades and
  environmental failures (unavailable toolchains, timeouts, errors) are unchanged and still fail strict.
- **jni**: request the core crate's configured feature set in the generated JNI `Cargo.toml`. Features were read only
  from `[crates.kotlin_android] features`, so a consumer that omits that key got a dependency on the core crate's
  default features while the generated shim still called into feature-gated modules — the crate then failed to compile
  with `E0433`. The lookup now falls back to the top-level `features`, matching every other binding scaffolder, and an
  explicit `[crates.kotlin_android] features` still wins.
- **extract**: apply a `#[cfg(feature = "…")] pub mod` gate to the items inside it regardless of which source file
  declares the module. Only `sources[0]` was scanned for module-level cfg attributes, but `sources` is an
  author-ordered list and the file holding the gated module is frequently not first. Every item under such a gate was
  recorded with no cfg, so backends that exclude items by cfg — notably wasm, whose target cannot compile
  non-wasm-safe modules — emitted calls into modules absent from their feature set.
- **ffi/all**: rebuild a missing or stale cbindgen header after all FFI source-writing stages and validate the refreshed
  declarations in the same `alef all --clean` run, instead of failing once and requiring a manual Cargo build before
  an identical second generation could succeed.
- **e2e/rust**: accept `crates.e2e.error_field_aliases` and apply the configured mapping when generated Rust tests
  assert fields on an error value.
- **jni**: honor `crates.jni.exclude_functions` when emitting top-level and instance-method shims, alongside the
  paired Kotlin and Kotlin Android exclusion lists.
- **scaffold/php**: emit exactly one composer.json per layout. The co-located layout now emits only the
  repository-root manifest; the package-directory copy is kept for the split layout, where it is the
  installable package. Both rendered the same composer `name`, so every co-located consumer carried a
  second, unreachable declaration of its own published package — Packagist reads the repository root, and
  consumer references target the class directory rather than the manifest.
- **ffi**: preserve registration methods' domain error types when a service owner is absent, and return the scalar
  zero sentinel when an opaque constructor rejects an enum discriminant.
- **e2e/snippets**: assert the whole rendered snippet document in the canonical-language test instead of a set of
  substring probes, so `level`, `requires` and `side_effect` are pinned again; a renderer emitting a bogus value for
  any of them previously satisfied the probes.

- **snippets/check**: build Rust snippet sessions with the crate's declared features in `alef snippets check`, the
  same merge `alef docs` already performs. The command dropped them, so the path dependency in the generated
  snippet-check manifest resolved with default features only and every snippet importing a feature-gated module
  failed with `unresolved import`.
- **e2e/snippets**: declare the `serde_json` dependency that generated Rust snippet bodies name. A `json_object`
  argument makes the Rust recipe emit `serde_json::from_str(…)`/`serde_json::from_value(…)`, but the snippet
  frontmatter and coverage ledger both reported `requires: []`, so the Rust snippet validator built its check
  project without the crate and every such snippet failed with `E0433: failed to resolve: use of undeclared crate
  serde_json`. Rust snippets now carry a `crate:serde_json` requirement, and the Rust validator resolves
  `crate:<name>` requirements into `[dependencies]` of the check project. Session configuration still wins: a crate
  declared under `docs.snippets.sessions.<target>.rust_dependencies` keeps its configured version and features, and
  a requirement Alef has no pinned version for fails with the config key to add instead of resolving silently.
- **ffi**: verify cbindgen declarations against the FFI source after writing it, so a stale-header failure leaves Cargo
  the new source it must rebuild instead of trapping generation in a rebuild loop; declaration matching also accepts
  formatted line breaks while retaining exact symbol boundaries, and includes exports from generated modules such as
  `service.rs` rather than treating their valid header declarations as removed.
- **codegen**: preserve public associated serde default providers as callable Rust paths instead of treating every
  `#[serde(default = "Type::function")]` as private and attempting structural JSON recovery, which failed when the
  owning configuration contained required nested named fields.
- **snippets**: stop the shared batch timeout from truncating to zero for the first validator in a batch.
  `remaining_batch_timeout` floored the time left via `Duration::as_secs`, so the near-full budget the very first
  caller sees (a few elapsed nanoseconds short of a whole second) rounded down to 0 and the freshly added
  zero-budget guard rejected every snippet in the batch without running any of them — a batch validated nothing,
  silently. The remainder now rounds up to the next whole second instead of down.
- **e2e/ruby**: compare `equals` assertions exactly. Ruby stripped both the actual and the expected value, which is
  symmetric and so never produced an unsatisfiable assertion, but it also made a genuine trailing-whitespace
  regression invisible in Ruby while every other backend compares exactly. String coercion is kept; normalization is
  not.
- **e2e/elixir**: stop normalizing the actual value in generated `equals` and `is_empty` assertions. `is_empty`
  emitted `String.trim(actual) == ""`, which passes for a whitespace-only value that Python's falsy check and
  TypeScript's length check both reject — so one fixture assertion disagreed across languages.
- **e2e**: compare `equals` assertions exactly in generated Python, PHP, Rust, TypeScript and WASM tests. The actual
  value was normalized with `.trim()`/`.strip()` while the fixture `expected` literal was emitted verbatim, so a
  fixture whose expectation legitimately ends in a newline could never be satisfied, and a genuine trailing-whitespace
  regression was silently absorbed. Neither side is normalized now.

- **magnus**: marshal `initialize`'s keyword types the same way as the accessors. The kwargs constructor
  converts each field with `<mapped type>::try_convert` and `Json` maps to `String`, so a `json_value`
  keyword promised a parsed document the constructor cannot accept — `try_convert` yields `None` on a Hash
  and the field silently falls back to its default rather than raising.
- **magnus**: declare `.rbs` attributes read-only. `attr_accessor` was emitted for every field of a defaulted
  struct, but the binding defines no writer for any field, so steep green-lit assignments that raise
  `NoMethodError`.
- **magnus**: declare each `.rbs` attribute as the accessor the extension actually emits. `Json` is mapped to
  `String` recursively, because the binding serializes it before Ruby sees it — `json_value` promised a parsed
  document that never arrives, so steep accepted `page.extracted_data["key"]` on a `String`. Nullability now
  follows the field's own optionality rather than the owning type's `has_default`, which had been nil-wrapping
  accessors that can never return nil.
- **e2e/rust**: share one field-aware containment predicate across `contains`, `contains_all`, `not_contains`, and
  `contains_any`, so enum and collection fields no longer emit assertions that fail to compile. Only `contains`
  handled those field kinds; the other three emitted a plain `.contains()` call that an enum does not have and
  that compares whole collection elements. `contains`'s own output is byte-for-byte unchanged.
- **snippets**: preserve extension-owned fixture descriptions while validating generated documentation language
  identities.
- **readme**: name the missing `crates.readme.snippets_dir` (or `snippets.<key>`) config again when a README
  template's `include_snippet` call references an undefined snippet mapping, instead of surfacing serde's generic
  "did not match any variant of untagged enum" message. Undefined values must be rejected before struct
  deserialization of the (String | {path, root}) snippet mapping, which fails silently on the underlying cause.
- **napi**: allow string-enum field literals alongside the nominal TypeScript enum using values derived from the
  canonical enum emitter.
- **kotlin-android**: treat a DTO as default-constructible only when every emitted constructor parameter has a
  Kotlin default, including transitive nested DTO defaults.
- **dart**: keep config parameters optional for serde function defaults by delegating to generated JSON
  construction, preserving the source default instead of synthesizing a zero value.
- **rustler**: JSON-encode default-valued records at public boundaries, report malformed payloads with context, and
  preserve async, error, and fluent-resource return shapes.
- **generate/verify**: stamp every emitted file carrying an Alef marker, including backends that template their own
  marker while intentionally leaving `generated_header` disabled.
- **ffi**: fail generation when generated FFI exports and the on-disk cbindgen header come from different runs.
- **ffi**: declare the callback-style streaming method wrapper's `client` parameter over the same scalar
  `AlefHandle` every producer of that client type returns, instead of a `TYPE *`/`const TYPE *` struct pointer.
  Every FFI producer hands out client types through `insert_handle`, never `Box::into_raw`, so the mismatched
  pointer parameter made the generated Rust and the generated C header describe two incompatible shapes for the
  same handle, and no caller holding a valid client handle could invoke a streaming method without a cast that
  was wrong by construction.
- **ffi**: declare enum `_free`/`_to_json`/`_to_string`/`_from_json` companions over the same scalar `AlefHandle`
  every producer returns for that enum, instead of a `TYPE *`/`const TYPE *` struct pointer. Every FFI producer
  (function returns, method returns, field accessors, and JSON constructors) hands out `Named` types — enums
  included — through `insert_handle`, never `Box::into_raw`, so the mismatched pointer signature made the
  generated C header describe two incompatible shapes for the same handle and broke consumer codegen (e.g. Go
  `cannot use ... as *_Ctype_struct_... value`) for any enum with data-carrying variants.
- **codegen/conversions**: use one tuple-variant predicate for enum definitions and `From` conversions so
  adjacently tagged tuple variants and untagged struct variants emit matching Rust syntax (#232).
- **ffi/service**: free the C-allocated response pointer before deserializing it in the generated service handler
  bridge, so a malformed response no longer leaks the buffer when parsing fails.
- **e2e/python**: stop emitting broken generated snippets/tests for `json_object` args configured with
  `options_via = "from_json"`. Construct via the type's plain kwargs constructor instead, stop importing that type
  from both the public module and the native bindings module in the same file, and bind the call result whenever
  the snippet template is going to print it instead of discarding the return value and printing an unbound name.
- **e2e/snippets**: give the Python, Go, Dart, and TypeScript snippet generators a crate-name-derived
  `"{PascalCase(crate name)}Error"` fallback for the error type they import/catch, instead of the bare literal
  `"Error"`, which almost never names a real generated type. Scoped to the four snippet emitters via a shared
  `snippet_error_type_name` helper — `ResolvedCrateConfig::error_type_name()` itself is unchanged and still
  defaults to `"Error"`, since 11 Rust-generating backends (extendr, rustler, wasm, php, magnus, ffi, swift, pyo3,
  jni, napi) consume it through `error_constructor_expr()` to generate Rust, and some consumer crates genuinely
  name their error type `Error`.
- **snippets**: enforce one timeout budget across every snippet in a validation batch and terminate timed-out
  toolchain process groups so descendant processes cannot keep docs generation alive.
- **cache**: serialize IR maps and sets in canonical order so unchanged inputs retain stable IR and backend cache hashes.
- **codegen/defaults**: stop emitting `#[serde(default = "path")]` functions as callable initializers in generated
  Rust (Magnus, PHP, NAPI, Rustler) — the named function belongs to the source crate, is not `pub`, and is frequently
  `#[cfg(feature = "serde")]`-gated, so the binding failed to compile with `E0425: cannot find function`. Generated
  Rust now recovers the field's real value by deserializing a minimal JSON stub through the source type's own
  `Deserialize` impl, the same mechanism `#[serde(default = "path")]` itself relies on. Where the owning type is not
  known, generation fails with the crate, type, field, and uncallable function named, rather than substituting the
  field type's zero value — which compiles and looks right while disagreeing with the source crate's configured
  `default_span()` is `1`; `u32::default()` is `0`).
- **validate versions**: discover nested C#, Dart, Zig, and Cargo lock manifests, validate all C# assembly version
  fields and every local lock package against its manifest, and normalize doubled path separators in diagnostics.
- **ffi**: preserve fully qualified streaming request types when emitting handle validation and lookup code.
- **generate/all**: stamp each successful generation stage before later work can fail, and defer standalone orphan
  cleanup until post-build succeeds while preserving non-header generator and scaffold outputs.
- **snippets**: isolate per-target validation-session preparation failures so healthy language sessions still run
  while strict validation reports the affected target as an error.
- **snippets/c/go**: model public FFI handles as scalar `AlefHandle` values and use zero as the invalid-handle sentinel.
- **pyo3/e2e-python**: unify the two independent `from_json` eligibility checks — pyo3's `#[pymethods]`
  injection/`.pyi` stub gate read crate-level serde availability only, while the e2e python snippet emitter's
  gate read per-type serde derives only — into one shared `pyo3_from_json_eligible` predicate
  (`src/codegen/conversions/helpers/eligibility.rs`) requiring per-type serde derives, crate-level serde
  availability, and core<->binding convertibility. The two gates could disagree for a crate whose types carry
  serde derives but whose Python binding crate lacks `serde`/`serde_json`, or vice versa; both call sites now
  delegate to the shared predicate instead of each re-deriving it.

- **fixtures/readme**: separate reader-facing fixture inputs, result presentation, and error intent from test data,
  and allow individual README snippet mappings to migrate between roots without breaking sibling mappings.
- **e2e/python**: emit unbound calls when a fixture only verifies that the call does not raise, avoiding unused-result
  Ruff failures during generation.
- **snippets/c**: derive list and map return ownership from the Rust IR and use the generated JSON-string ABI.
- **e2e/kotlin/swift**: make `not_empty` assertions distinguish nullable scalars from nullable containers.
- **e2e/rust/zig**: generate mutable JSON patch values and use a runtime Zig I/O provider for file inputs.
- **e2e/dart**: emit trait-stub wrapper factories when test backends come from call-level argument mappings.
- **e2e/typescript**: lower IR byte arrays and enum wire strings to their typed WASM values.
- **generate/snippets**: propagate mandatory post-build failures from `alef generate`, terminate descendant validator
  processes at the configured timeout, and report stable generated-file totals across content-identical passes.
- **snippets**: use built-in language recipes for declarative trait-bridge fixtures and resolve C registry operations
  from configured bridge identities through canonical ABI naming.
- **snippets/c**: generate compilable trait-bridge examples with IR-derived callbacks, initialized vtables, canonical
  registration calls, and owned userdata cleanup.
- **e2e/rust**: emit Clippy-clean mock-server route loading.
- **ffi**: return named values as generational handle tokens and resolve nested Rust type paths without duplicating
  the core module.
- **clean**: use the generated Kotlin Android Gradle wrapper and avoid invalid generic Dart and C# clean commands.
- **e2e**: keep ordinary generation from writing a fixture schema before validation succeeds.
- **snippets/docs**: emit reader-facing, fail-closed examples, recognize Astro content-collection references, and
  allow per-language README snippet roots.
- **sync-versions**: preserve README content unless regeneration is explicitly requested with `--regen`.
- **wasm**: delegate instance methods with borrowed named inputs to the Rust core and compile C environment shims
  only for `wasm32` targets.
- **ffi**: keep named field accessors on scalar handle tokens, fail closed for lifetime-borrowed types that cannot
  enter the process-global registry, preserve forward-compatible error taxonomy matching, and type empty handle
  acquisition lists explicitly.
- **ffi/csharp**: retain configured trait registration exports and matching P/Invoke declarations when visitor
  callbacks also bind the trait through an options field.
- **ruby**: exclude compiled native libraries, object archives, logs, and debug-symbol bundles from scaffolded source
  gems.
- **zig**: derive the scaffolded local FFI header include directory from the configured FFI output path.
- **snippets**: emit strict TypeScript DTO literals and optional accessors, prefix WASM imports, and deserialize
  Kotlin Android inputs using each argument's declared DTO type.
- **node**: export zero-argument adjacent-enum namespace constructors as callable functions instead of getters.
- **csharp**: emit formatter-stable imports, native calls, and sealed-union converters that pass `dotnet format`.
- **zig**: keep generated bindings silent, validate nullable native returns before dereference, and release owned native
  buffers even when Zig allocation fails.
- **swift**: emit JSON constructors for serializable types referenced by generated API signatures, including opaque
  configuration types used by setters.
- **node/python**: attach stable numeric taxonomy codes to generated native error conversions while preserving typed
  Python exception classes and generic fallbacks.
- **java**: emit native-library required symbols one per line so generated bindings satisfy Checkstyle line limits.
- **java/kotlin**: map stable native taxonomy codes to generated typed exceptions while retaining generic FFI error
  fallbacks.
- **go/zig**: map native failures to typed binding errors by stable numeric FFI taxonomy codes instead of parsing
  human-readable error messages.
- **Java/C#**: require either a shared-native-runtime contract or an explicit borrowed-static, ABI-compatible,
  no-destructor capsule contract before wrapping native pointers, and protect C# service calls and callback
  registrations with SafeHandle lifetime guards.
- **C#**: restore readable formatting and a valid `/// <summary>` doc comment block in the service templates whose
  SafeHandle lifetime guards were reflowed onto run-together lines, including a bare `</summary>` that made the
  generated source uncompilable.
- **ffi**: report stable, collision-checked per-variant error taxonomy codes while preserving reserved conversion and
  panic codes.
- **JNI/Kotlin Android**: transfer configured capsule values as their raw host pointers instead of boxed Rust
  wrappers, omit incompatible Alef destructors, and make generated opaque-handle closure synchronized and idempotent.
- **generate**: preserve unchanged generated files and modification times across clean regeneration, reconcile only
  manifest-owned orphans, keep handwritten scaffold files outside formatting and hashing, and retain managed hashes
  on generated JNI manifests.
- **verify**: evaluate fixture-snippet coverage from current fixtures and renderers, reject stale or malformed coverage
  ledgers and missing tracked snippets, and preflight `alef all` before generated-file writes.
- **IR**: attach serialization-compatible, deterministic error type, variant, and numeric code taxonomy metadata to
  every extracted error variant.
- **extract**: preserve serde function defaults and supported zero-argument default calls as explicit runtime
  providers instead of collapsing them to empty language defaults.
- **extract**: suppress inherent impl methods and generic-method diagnostics when their declaring type is excluded.

- Fixed indexed JSON-scalar metadata matching, simple-result enum containment, falsifiable collection containment,
  and JNI streaming exports for opaque owner handles without ordinary methods.

- Made Rust and Kotlin containment assertions respect the effective result type, including complex arrays and
  nullable JSON values, without applying whole-record debug matching from unrelated field metadata.
- Imported streaming request DTOs into generated Kotlin tests and verified Start/Next/Free JNI declarations and
  exports in both directions.

- **snippets**: fail E2E generation when an expected documentation snippet has no compatible built-in or
  extension recipe, including cached coverage manifests, instead of warning and crediting an incomplete corpus.
- **snippets**: use pyrefly as the sole built-in Python snippet type-checker, matching scaffolded project tooling and
  reporting it explicitly when unavailable.

- **cli**: report disk-scanned orphan candidates instead of deleting them. The reclaim rule infers "orphan" from
  "absent from this run's output", and three situations produce that absence — the emitter stopped emitting the
  file, the emitter failed to emit it, or it is a create-once seed alef emits only when absent. No clause separated
  them; the manifest clause certifies that the backend keeps books, not that this file was deliberately dropped. The
  marker likewise records when alef last wrote a file, not whether alef owns it, so the rule protected neglected
  trees and endangered current ones: on a tree where 159 of 160 files carry markers, the public entry point is a
  candidate. Only a positive assertion from the producer can separate "dropped" from "failed to emit", and no such
  record exists yet, so the disk-scan route now reports.
- **e2e**: do not prune orphaned snippets on incomplete coverage. The prune ran before the completeness gate, and
  that gate only defers its error, so ordering alone would not have helped. A snippet that merely failed to render
  is absent from `generated_paths` while its language is still expected — indistinguishable from a genuine orphan —
  so a transient generator failure unlinked published documentation. `orphaned_paths` stakes its whole safety
  argument on that gate having passed first.
- **cli**: never prune a `poly.toml` table a scoped run did not emit. The prune step read a path absent from this
  run's output as an empty array, so a scoped `--lang java` generate stripped the consumer's whole rule selection
  down to `select = []`, which every linter accepts and then checks nothing. Only a path this run did emit can
  testify that one of its values is gone.
- **cli**: record every language manifest `alef all` writes. The service-API, stub and public-API phases wrote files
  and stamped hashes without folding them into the language manifest, so a backend recorded a single path while 38
  generated modules sat unrecorded on disk — and absence from the manifest reads as "this backend emits one file".
  Both manifest writers now log the crate, language and path count, since a pathologically small manifest was
  previously indistinguishable from a backend that genuinely emits one.
- **docs**: emit every page before validation can fail the stage. Snippet discovery and validation were fused into
  one call that ran before the CLI, MCP, `llms.txt` and `SKILL.md` emitters, so any strict bail, gap failure or audit
  failure returned before those pages were pushed — and downstream, a page missing from that list is read as one
  alef no longer emits. The stage's documented promise that its file list survives a failure covered only the first
  pass.
- **docs**: stop idiom-translating rustdoc intra-doc link targets. `rust_links_to_plain` only degraded links whose
  text began with a backtick, so a plain-text intra-doc link fell through to `rust_paths_to_dot_notation`, whose
  blanket `::` → `.` substitution rewrote the link *target* into a relative Markdown link that resolves nowhere: 26
  MD057 errors across 13 reference pages on one consumer. Both passes were individually reasonable and wrong in
  composition. Links are now degraded only when the target is identifier-shaped and is not anchored, schemed,
  slash-bearing or `.md`/`.html`-suffixed, and tests pin both directions.
- **zig**: declare every error the emitted body can return. `wrapper_return_type` and `method_return_type` each
  carried their own list of which return shapes need `OutOfMemory`, and neither matched a bare opaque-handle return
  — while both body emitters unconditionally emit `if (_result == 0) return error.OutOfMemory` for exactly that
  shape, so four generated functions declared `error{HandleClosed}` and returned `OutOfMemory` and the binding did
  not compile. The shape is derived once for both callers, and an assertion after emission checks that the declared
  set admits every `error.X` the body returns.
- **csharp**: derive the null-check sentinel from the P/Invoke return type. A capsule return was declared
  `extern ulong` while the FFI crate exports it as a raw `*const T` — Dart and Zig both agree on the pointer — so
  the wrapper's `== IntPtr.Zero` check was correct and the signature was wrong. The declaration and its sentinel now
  come from one function that reads the declaration string itself. Affects only types listed in `capsule_types`,
  which is why the common path looked flawless on a tree that configures none.
- **dart**: derive a snippet's call shape from the binding's own predicate. The snippet emitter decided whether a
  config argument was named or positional from a hardcoded list of type-name substrings while the binding decided it
  from whether a default expression can be synthesized, so for six type families the binding declared named-optional
  and the snippet called positionally and did not compile.
- **e2e (kotlin)**: bind a streaming adapter's request before the snippet uses it. The docs-snippet emitter resolved
  the owner handle but never the declared request parameter, so the emitted snippet either called the receiver with
  no arguments or passed raw handle-config-contaminated JSON where the typed request belonged. `kotlin_android`
  delegates here and was affected too.
- **e2e**: honour `exclude_functions` in the snippet coverage ledger. The expected set was built without consulting
  per-language exclusions, so it expected fixtures the emitter is configured never to produce, recorded them as
  generated, reported `missing: []` while those paths were absent from disk, and the tracked-file check then killed
  the run on a discrepancy the ledger had declined to record. Absent tracked paths are now all reported with a count
  instead of bailing on the first, and are named as distinct from what `missing` explains.
- **snippets**: register alef's ownership of the coverage ledger at write time. The ledger is strict JSON so it can
  never carry a provenance marker, and nothing recorded ownership, so every rewrite was refused, the stale copy
  survived, and the tracked-file check then failed the run on contents alef had just been prevented from refreshing.
  `alef adopt` could not repair it either: with no marker it classified as a create-once seed, the category for files
  a human grows past a placeholder. The write guard and adopt now share one notion of ownership; hand-written files
  at unregistered paths are still refused.
- **snippets**: keep validator scratch inside the cache root and sweep it. An earlier pass moved scratch under
  `.alef/snippets/tmp` but left the sweeper looking in the working directory, so the safety net pointed at the empty
  set and files survived every run. Cleanup is now an RAII guard, because validators return from four kinds of place
  and the explicit calls covered some of them; its `Drop` retries, since a killed child can still be exiting and lose
  the race with `ENOTEMPTY` — which `TempDir` discards silently, making a leak indistinguishable from success.
- **scaffold**: exclude the e2e snippet output tree from poly discovery. `docs_snippets_excludes` read only
  `[workspace.docs.snippets] dirs`, so a consumer configuring `[crates.e2e.snippets] output` alone got no protective
  exclude for the tree alef writes generated snippet Markdown into. The hazard is latent — in every tree surveyed the
  two keys name the same directory — but the divergent-key case is real.
- **codegen**: name the owning type by its full path in a `compile_error`. The diagnostic spliced the crate from
  `rust_path` onto the type from `name`, dropping every intermediate module and rendering `demo::inner::Settings` as
  `demo::Settings`. A diagnostic that misnames the definition it points at sends the reader to the wrong place.
- **c**: pair a snippet's failure guard with the declaration it names. The guard was emitted by a positionally-blind
  pass that replaced any assert line, so a client-construction assertion above the call became a guard on a result
  variable above its declaration. A single walk now carries declaration state, so the guard cannot precede the
  declaration it reads and an out-of-order match becomes a generator diagnostic instead of published C. Output is
  byte-identical for every currently valid snippet.

### Removed

- **snippet validation**: remove the legacy path-only `alef snippets validate` command, the fail-whole-map session
  preparation API, and the report-dropping snippet artifact projection. Use configured `alef snippets check`,
  isolated session preparation, and `generate_snippet_report` so validation retains sessions, coverage, audits, and
  missing-generation diagnostics.

### Changed (BREAKING)

- **FFI handles**: ordinary opaque values now cross the C ABI as scalar, generational `AlefHandle` tokens with zero as
  the invalid sentinel. Regenerate every host binding and C consumer; pointer-shaped calls to constructors,
  accessors, serializers, streaming methods, and destructors are no longer compatible.
- **capsules**: Java, C#, JNI, and Kotlin Android capsule mappings must explicitly describe either shared-runtime
  ownership or borrowed-static ABI compatibility. Owned, refcounted, and WebAssembly-backed pointers remain
  fail-closed unless the configured host contract can preserve their lifecycle.

## [0.60.2] - 2026-08-12

### Fixed

- Kotlin E2E generation now constructs streaming adapters' declared request DTOs instead of passing primitive fixture
  arguments to typed owner methods, and cross-checks streaming native declarations against generated JNI exports.

- Generic documentation snippet generation now records calls with no effective function identity as missing coverage
  instead of emitting invalid empty calls and counting those files as generated.

- Generated Rust E2E assertions now use declared collection and enum field metadata for textual containment checks,
  avoiding invalid string arguments to `Vec<Named>::contains` and nonexistent enum `contains` methods.

- Restored structurally corrupted FFI, Go, Swift, Zig, and Node templates while preserving their allocator,
  ownership-transfer, concurrency, native-symbol, and runtime-export semantics.

- Generated FFI service modules now define their own panic guard, and generated Rust headers are placed before
  multiline inner attributes instead of being inserted inside their token trees.

- Snippet session cleanup now tolerates scratch directories removed concurrently and reports the exact path and
  operation for other filesystem failures, preventing opaque intermittent `ENOENT` failures in docs and snippet checks.

- E2E formatting now resolves generated language directories before changing the formatter working directory,
  preventing relative paths from becoming nonexistent doubled paths while rejecting formatter engine failures that
  older Poly versions only reported as warnings.

- Standard-library trait implementations on structs are no longer extracted as public binding methods, preventing
  methods such as `Debug::fmt` from producing lossy sanitized APIs and blocking generation.

- Generated Elixir test helpers now rely on `System.put_env/2` and no longer call the intentionally omitted
  `set_env` NIF.

- Generated Ruby bindings now use tuple constructor and match syntax for adjacently tagged positional enum variants.

- Kotlin assertions over optional JSON scalar fields now stringify safely instead of invoking string-only extensions
  on `Any?` values.

- Generated Rust documentation snippets now preserve every line inside multiline raw string literals instead of
  dropping or reindenting literal contents, and no longer retain the surrounding test module's closing brace.

- Generated Java bindings now honor explicit native-library path overrides before bundled resources and report every
  missing ABI symbol, the exported count, and the loaded path in one eager startup diagnostic.

- Generated agent skills now include required YAML `name` and `description` frontmatter when templates omit it.

- Alef now warns when the running CLI is newer than `alef.toml`'s pinned version, making the pin update visible
  before regeneration.

- Generated Java E2E assertions now retain statement separators when multiple assertions share a test method.

- Generated C# native declarations now retain data-enum handles while excluding unit enums and traits, preventing
  both missing live FFI declarations and calls to nonexistent JSON or destructor exports.

- Generated C E2E harnesses now propagate assertion failures into per-test results, return a failing process status,
  and report credential-gated tests as skipped instead of silently counting every invocation as passed.

- Kotlin JNI bridge declarations now exclude sanitized methods and only expose destructors for opaque handles returned
  by emitted functions, keeping every declared native method paired with a generated JNI export.

- Generated JNI shims now gate feature-dependent functions by their target-specific dependency feature sets, keeping
  disabled APIs out of Android builds while retaining enabled fallback implementations.

- Generated JNI manifests now inherit dependencies declared by the consumer workspace, keeping binding crates aligned
  with workspace dependency versions while retaining standalone fallbacks.

- Generated Python data-enum accessors now follow Serde's externally tagged wire shape, including bare-string unit
  variants and renamed payload keys, instead of assuming every enum contains a `tag` field.

- Generated Java opaque handles now make `close()` idempotent, clear native ownership before freeing, and reject method
  calls after close instead of allowing double-free crashes.

- Generated ownership headers now recommend `alef verify` directly instead of the deprecated no-op `--exit-code` flag.

- Go methods on value types now emit receiver-marshalling statements on separate lines, restoring valid generated Go
  syntax while preserving consumed-receiver ownership.

- Zig opaque-handle methods now convert C ABI integer booleans to Zig `bool`, preventing generated methods from
  returning an incompatible `i32` value.

- Generated Zig E2E tests no longer ignore `SIGABRT`, so allocator corruption and native aborts fail the suite instead
  of being silently suppressed.

- C FFI string-length companions now keep function-specific thread-local lengths, preventing intervening calls from
  turning a valid returned string into an out-of-bounds slice. Feature-gated opaque constructors now match their
  destructors and header declarations, while field accessors report conversion failures and document pointer ownership.

- JNI shims now contain panics across every Rust-owned JVM entrypoint and reject zero `jlong` handles before
  constructing Rust references, preventing unwinds and null-reference undefined behavior at the JNI ABI boundary.

- E2E regeneration now refreshes `fixtures/schema.json`, keeping consumer fixture schemas aligned with structured
  documentation metadata supported by the generator.

- Batched snippet validation now drains compiler output concurrently to prevent large-corpus pipe deadlocks, reports
  batch start and completion, keeps temporary workspaces under `.alef`, and removes timed-out legacy root-level
  scratch directories on subsequent validation runs.

- Snippet gap detection now discovers imported snippet content in Astro component files as well as MDX pages.

- `alef test` now fails when preconditions skip every explicitly requested language instead of reporting a vacuous
  success with zero executed suites.

- Snippet validation limits now cap the effective validation level instead of skipping the snippet when a stronger
  level is requested; bare fences remain unannotated and validate at the configured level.

## [0.60.1] - 2026-08-12

### Changed

- `alef verify` now fails on stale bindings or version drift by default; use `--report-only` for advisory output. The
  deprecated `--exit-code` flag remains accepted as a no-op for compatibility.

- Development version advanced to 0.60.1 for the strict snippet-validation and migration fixes.

### Fixed

- Serialize JVM-backed snippet validator integration tests so concurrent Java and Kotlin compiler startup cannot
  destabilize the JDK module image during the full test suite.

- Preserve line boundaries in generated JNI crate headers so Rust attributes, imports, and constants remain valid syntax.

- Keep FFI panic-guard and JNI unsafe-lint audits aligned with generated output.

- Emit every `not_contains` value in generated E2E assertions when fixtures use the plural `values` form.

- E2E fixtures can opt into preserving declared `mock_url` and `mock_url_list` values verbatim across generated
  language tests, so URL-policy and SSRF regressions exercise the address declared by the fixture rather than a
  substituted mock-server URL.

- Integration-test enum fixtures now initialize adjacent Serde content metadata, restoring full-suite compilation.

- E2E and registry test-app generation now fails when a configured formatter is unavailable or exits unsuccessfully,
  preventing noncanonical generated output from being reported as successful.

- Generated Ruby, PHP, R and Homebrew E2E error tests now assert the error value a fixture declares, matching it
  against either the error's message or its class name. PHPUnit's `expectException*` and testthat's `expect_error`
  combine message and class with AND, so both emit an explicit try/catch to express the disjunction.

- Generated Homebrew E2E error tests no longer interpolate the declared value inside a double-quoted `echo`, where a
  value containing `$` or a backtick would have been expanded by the shell.

- Generated Elixir E2E validation tests now call the operation under test when engine creation succeeds, instead of
  stopping after asserting `{:error, _}` on creation. Fixtures whose error is raised per-request rather than at
  construction previously asserted nothing and could never fail.

- Generated Elixir and Gleam E2E error tests now assert the error value a fixture declares, matching the reason's
  `inspect` rendering so a plain message and a typed atom or struct are both covered by one check.

- Generated Swift and Dart E2E error tests now assert the error value a fixture declares, matching it against either
  the error's description or its runtime type name. Swift's `catch` accepted any error and Dart used
  `throwsA(anything)`, so neither could distinguish the declared error from any other failure.

- Generated Java and C# E2E error tests now assert the error value a fixture declares, matching it against either the
  thrown exception's message or its type name. The existing expected exception type is unchanged; only the value check
  is added.

- Generated Kotlin and Kotlin Android E2E tests now dispatch streaming `owner_type` adapters as instance methods on the
  owner handle rather than as static facade calls, so the generated sources compile. The Kotlin backend had never
  ported the branch the Java backend already implements.

- Generated TypeScript and WebAssembly E2E error tests now assert the error value a fixture declares, matching it
  against either the error's message or its `name`. `.rejects.toThrow(regex)` only inspects the message, so the
  disjunction is expressed with a `toSatisfy` predicate; declared values are escaped for regex-literal context.

- Generated Go E2E error tests now assert the error value a fixture declares, matching it against either the error's
  message or its concrete type, instead of only checking that some error occurred.

- Generated C E2E error tests now fail when the call unexpectedly succeeds. Fixtures whose call used a
  `raw_c_result_type` emitted no error check at all, so the test passed regardless of outcome. Unmodeled result types
  fall back to asserting a non-zero `last_error_code()` rather than emitting nothing.

- Generated Zig E2E error tests now fail when the call unexpectedly succeeds. The previous shape wrapped the call in
  `catch { try testing.expect(true); return; }`, so a successful call skipped the catch entirely and the test passed
  having asserted nothing, while `expect(true)` was a tautology on the error path.

- Run the configured E2E formatter pipeline on cached test-app outputs so formatter and configuration updates converge
  without requiring a clean regeneration.

- Generated API examples now emit valid empty-map literals for Elixir and R, parameter fallback text is complete for
  unnamed values, and generated package READMEs carry generated-file headers.

- Generated reference pages now omit members gated by `cfg(test)` even when a binding enables an umbrella `full`
  feature.

- Generated Markdown tables now preserve square brackets inside code spans while escaping brackets that would form
  reference links, avoiding malformed API type cells.

- Generated parameter documentation now retains multiline rustdoc `# Arguments` descriptions and accepts all
  CommonMark unordered-list markers.

- C and Zig documentation snippet sessions now accept configured native include directories, and Swift validation
  tolerates SwiftPM binary directories that have not been created yet.

- Kotlin documentation snippets now implement configured visitor callbacks and attach them to conversion options.

- Rust documentation snippets now preserve raw-string delimiters and declare visitor feature requirements; docs
  validation enables the crate's configured Rust features for local snippet sessions.

- Go documentation snippets now emit visitor implementations and pass visitor-backed options to binding calls.

- Java documentation snippets now construct configured visitors and attach them through generated options builders.

- C# documentation snippets now instantiate configured visitor implementations and attach them to conversion options.

- C visitor documentation snippets now use only public binding APIs and omit test-harness JSON assertion helpers.

- Elixir documentation snippets now place visitor callbacks inside the conversion options argument.

- Dart documentation snippets now import and initialize the generated Rust library and dispose it on every exit path.

- Enum extraction now preserves Serde `content` metadata for adjacently tagged wire representations.

- Generated Go and Node bindings now preserve adjacent enum tag/content fields and reject unknown Go discriminants.

- Generated Node bindings now export adjacent enum unit values and payload factories through runtime namespaces.

- Generated Ruby and Elixir bridge enums now retain adjacent Serde tag/content shapes, including tuple payloads.

- Generated TypeScript visitor snippets now use lowercase wire actions, adjacent custom payloads, and trait-order code
  block arguments.

- Kotlin documentation snippets now import configured packages, shorten fully-qualified facade names, and only use
  coroutine entry points for asynchronous calls.

- PHP documentation snippets now enable strict types and load Composer dependencies before using generated classes.

- Ruby documentation snippets now require the load-path entry instead of the hyphenated gem distribution name.

- Zig documentation snippets now render directly as executable bodies, retain owned-result cleanup, and avoid test-only
  aliases and unmatched delimiters.

- Go documentation snippets now honor shared pointer-option configuration and print structured result fields.

- Generated-file manifests are deterministic and newline-terminated, empty manifests invalidate the cache, and
  `alef verify` now detects edits to individual generated file bodies.

- Rustler binding structs now raw-escape Rust keyword field names such as `type`, including constructors and tagged
  enum conversions.

- Generated Python and R visitor snippets now serialize callback actions with canonical lowercase wire tags and the
  adjacent `output` payload expected by visitor bridges.

- Generated WASM options-field visitor bridges now forward configured visitor handles instead of replacing them with
  `None` in input builders.

- Generated E2E assertions now preserve raw and whitespace-sensitive string semantics, enforce case-sensitive C#
  containment, validate content for `not_empty`, and fail null values instead of allowing vacuous matches.

- Snippet validation sessions now accept repository-relative native include paths, allowing Zig and C bindings that
  import generated C headers to compile without project-specific validator behavior.

- Swift snippet validation no longer reports an internal I/O error when SwiftPM's reported binary directory has not
  been materialized yet.

- Generated Elixir NIFs no longer expose a process-wide environment mutation helper, avoiding unsafe concurrent
  `setenv` access from BEAM scheduler threads.

- Generated Go wrappers no longer free marshalled value receivers after an owned-receiver FFI method consumes them.

- Generated FFI accessors no longer expose borrowed non-clone fields as owned handles that callers could invalidly free.

- Generated Swift trait protocols now document their concurrency contract, and their Rust `Send`/`Sync` assertions carry
  explicit safety invariants.

- Visitor callback payloads now use their reported byte length and an allocator-matched host destructor instead of
  reconstructing host allocations with Rust's global allocator; Go, C#, and Java callback tables provide the destructor.

- Generated cbindgen configuration now maps correctly escaped Rust feature predicates to prefixed C macros so
  feature-gated declarations remain guarded in public headers.

- Generated JNI service lifecycle, registration, and destructor entry points now catch Rust panics before they can
  unwind across the JVM ABI boundary.

- Generated Zig trait callbacks now copy returned strings into allocator-matched storage, free undispatched results,
  release callback strings through the matching allocator, and return owned error text through `out_error`.

- FFI and Java bindings now omit `free_bytes` when no generated API can produce its allocation metadata, preventing
  unrelated pointers from being mistaken for byte-result allocations.

- Infallible complex-return trait callbacks now consume and free host `out_error` diagnostics before returning a safe
  default on non-zero callback status.

- Options-field visitor callbacks no longer emit an unattached generic trait bridge whose public destructor could free
  shared host state independently of the live visitor handle.

- Every generated FFI entry point now clears stale thread-local error state before execution, while error and return
  metadata accessors preserve the state they report.

- C# visitor declarations now use configured FFI prefixes, and C#/Java options-field visitor bindings omit generic
  registry and byte-destructor symbols that their FFI library does not export.

- Attaching a visitor to an FFI options handle now transfers ownership into one synchronized object, preventing multiple
  independent mutexes from aliasing the same mutable visitor; Go wrappers honor the transfer.

- Zig options-field visitor helpers now call the actual generated visitor constructor with the correct callback and
  handle types instead of emitting a phantom trait-specific symbol and mismatched free contract.

- Snippet gap and audit commands now discover complete coverage ledgers beneath configured snippet roots, so generated
  output directories are trusted without hiding orphaned handwritten snippets.

- Generated FFI and JNI crate roots now contain Rust 2024 unsafe implementation lints, allowing consumers to inherit
  strict workspace lint policy without warnings from generated glue.

- Managed TOML scaffold manifests now carry Alef provenance, preserve unknown user tables during structured refresh,
  and participate in `alef diff`; write-once scaffold seeds remain untouched.

- Snippet coverage ledgers now reject tracked files that resolve outside their configured generated root, including
  symlink escapes.

- C documentation snippets now construct whole-input typed DTO handles from the public JSON API, preserve file-backed
  byte inputs, and free owned handles instead of omitting required arguments or passing placeholder nulls.

- Standalone C documentation snippets now construct every JSON argument with its declared ABI type and derive opaque
  return handles from extracted function metadata instead of call-name guesses or shared placeholder option types.

- Nested C result accessors now infer optional opaque handle types from extracted struct fields when no explicit
  `fields_c_types` override is needed, avoiding generation-time panics for authoritative IR shapes.

- Rust documentation snippet validation now checks compatible uncached cells as binaries in one Cargo invocation,
  while retaining per-cell diagnostics, cache entries, session isolation, and deterministic result ordering.

- Documentation snippets for expected-error fixtures now render idiomatic executable failure handling in C#, Dart,
  Elixir, Go, Java, Kotlin, Python, and Ruby instead of presenting failing calls as successful examples.

- Harness-only trait bridge fixtures now require extension-owned public documentation recipes instead of publishing internal test
  stubs, including when the test backend argument comes from global call configuration.

- Fixed Kotlin documentation snippets referencing undeclared typed input variables when fixtures do not define file presentation metadata;
  DTO JSON now uses centralized nested serde wire names and an idiomatic local mapper name.

- Snippet session regressions now use isolated .NET restore state and tolerate cold Windows toolchains without
  weakening validation.

- Snippet validation sessions now integrate configured package manifests and toolchain roots across Rust, TypeScript,
  Go, C#, Java, Kotlin, Dart, Python, Swift, and Zig, with isolated absolute caches and explicit Rust dependencies.

- Generated C snippets now use configured ABI prefixes, explicitly typed scalar and void return shapes, while Java snippets present
  returned values and selected fields; C Doxygen output also escapes nested comment delimiters.

- Elixir DTO typespecs now retain generated enum modules inside lists, and PyO3 single-variant enum constructors avoid
  warning-producing one-arm matches.

## [0.60.0] - 2026-08-10

### Fixed

- Fixture-generated snippets now request type-check validation instead of being downgraded to syntax-only skips.

- Fixture-generated documentation snippets now include canonical validation frontmatter without exposing test inputs or assertions.

- C FFI capsule returns now use const-null failure sentinels, keeping generated Rust pointer mutability consistent.

- C FFI streaming iterator `_next` functions now keep their full bodies inside the panic guard, producing valid Rust.

- Alef skip extraction now parses attribute structure instead of token substrings, supporting `#[alef::skip]` without
  misclassifying similarly named or feature-gated serde attributes, and positional enum/newtype fields now preserve
  field-level exclusion metadata.

- Alef scratch workspaces and caches are ignored recursively, preventing nested validation artifacts from appearing
  as consumer repository changes.

- Release verification now accepts Java documentation snippets that stage typed JSON before deserialization and keeps
  enum conversion helpers warning-clean under strict Clippy.

- WASM tagged-enum conversion now maps hidden or binding-excluded core variants to the binding default instead of
  trapping.

- C FFI byte-buffer ownership no longer relies on caller-controlled vector capacity metadata.

- JNI and Kotlin Android generation now selects cfg-gated function variants from the target feature set before
  deduplication, preventing disabled APIs from referencing absent core modules.

- Swift async return-type `Sendable` extensions now emit in canonical name order, keeping repeated isolated generation
  byte-for-byte deterministic.

- Kotlin/JNI opaque clients now serialize native calls with `close()`, make repeated and concurrent closes idempotent,
  and reject method or stream creation after close before entering JNI.

- Zig opaque and streaming handles now clear their nullable pointer before teardown, making repeated free/deinit safe and
  returning `HandleClosed` locally on use after teardown.

- Every generated Rust-owned C FFI entrypoint now contains Rust panics, including constructors, destructors,
  conversions, accessors, services, callbacks, traits, visitors, streams, bridges, and support helpers; contained
  panics use the existing thread-local error contract and return each signature's established failure sentinel.

- PHP flat data-enum tags are now read-only and JSON construction rejects unknown tags before infallible core
  conversion, preventing malformed or future variant tags from reaching generated panic arms.

- Rust documentation snippets now use public presentation inputs without leaking E2E mock-server environment or
  private-network setup, while generated E2E tests retain their runtime harness.

- Node and WASM snippet sessions now extend configured TypeScript project manifests, so declared local packages resolve
  during strict validation while stable validation workspaces still replace each snippet's source.

- Restore generated ownership headers when a managed backend file omits its inline header, keeping `alef:hash`
  verification and orphan cleanup active after regeneration.

- Domain-shaped E2E fixtures now require extension-owned documentation recipes instead of falling back to generic
  function-call generators that could emit invalid or test-harness snippets.

- Snippet gap and audit checks now discover current generated coverage ledgers directly from configured snippet roots,
  treating only ledger-backed generated files as references while continuing to report manual orphan snippets.

- Configured C#, Java, Node, and WASM snippet validation sessions now reuse stable project workspaces, preserving
  local package linkage and compiler state across snippets instead of creating an isolated project for every block.

- Generated Rust mock servers now emit valid fixture field types when loading documentation-rich fixtures.

- E2E fixture validation now accepts the complete structured documentation metadata model, including target paths,
  typed presentation arguments, file inputs, and result operations.

- Python visitor bridges now honor internally tagged return-action dictionaries such as
  `{"type": "custom", "output": "..."}` while retaining legacy externally tagged payloads.

- C documentation snippets now read configured file inputs into byte-array fields of typed DTO JSON.

- Zig documentation snippets now read configured file inputs into byte-array fields of typed DTO JSON.

- Swift documentation snippets now read configured file inputs into byte-array fields of typed DTOs.

- R documentation snippets now read configured file inputs into raw-vector fields of typed DTOs.

- Python, Rust, Node, and WASM documentation snippets now read configured file inputs into typed DTO byte fields.

- Ruby bindings now box converted named fields in data-enum variants when the Rust core field is boxed.

- Ruby and Elixir documentation snippets now read configured file inputs into typed DTO fields.

- Java and Kotlin documentation snippets now read configured file inputs into byte fields of typed DTOs.

- Go, C#, Dart, and PHP documentation snippets now read configured file inputs into native byte values inside typed DTOs.

- Go native DTO snippets now pass defaulted fields through pointers to match generated binding struct types.

- Go documentation snippets now inherit configured options DTO types when fixture recipes omit an inline type.

- Go documentation snippets now materialize absent typed DTO arguments as values and align native struct fields canonically.

- TypeScript documentation snippets now use imported enum members and safely destructure optional result collections.

- Fixture documentation presentations can replace inline test data with validated local-file inputs, and generated
  snippets no longer expose mock-server harness details.

- Dart and PHP documentation snippets now construct known DTO arguments with native typed constructors instead of
  JSON round trips.

- Go documentation snippets now construct known DTO arguments with native struct literals instead of JSON round trips.

- Rust and TypeScript documentation snippets now render display values, typed DTO inputs, and optional first-result
  collections using idiomatic, strict-mode-safe syntax.

- Go documentation snippet bodies now avoid a non-canonical blank line at the end of fenced source.

- Generated Go documentation snippets now match canonical `gofmt` layout, including imports and error blocks.

- C# documentation snippets now construct known DTO arguments with native object initializers instead of JSON round trips.

- Shared binding conversion regressions now keep test modules after production items for strict Clippy compatibility.

- Coverage-ledger side-effect metadata now uses its typed serialized representation without retaining an obsolete
  Markdown frontmatter renderer.

- Generated fixture snippets now keep validation metadata in the authoritative coverage ledger while rendering clean
  Astro-facing fenced Markdown with language titles and only explicitly configured user prose.

- Structured fixture presentations now preserve Rust `Result` unwrapping and import every TypeScript DTO referenced by
  docs-specific typed arguments, so result access and overridden inputs remain compilable.

- Generated Go documentation snippets now separate package and import declarations into gofmt-compatible lines.

- TypeScript E2E fixture regressions now retain formatter-clean protocol metadata initializers.

- PHP and Ruby now box converted values when binding DTO fields map to core `Option<Box<T>>` fields.

- Poly scaffolding now merges Alef defaults into an existing `poly.toml`, preserving custom tables, rules, excludes,
  and comments across clean regeneration while keeping repeated scaffold passes idempotent.

- Fixture documentation now supports typed input and argument overrides plus structured result presentation, allowing
  backend-owned snippets to render idiomatic field display and collection iteration without embedding language code.

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
  of dropping or mistyping the assignment path. (`src/backends/php/gen_bindings`)
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
  and the new `package_json_is_private` fail open — a missing or unparsable manifest counts as
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
  ruff release could silently start firing a new deny-by-default copyright-header rule
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

- **dart native loader emitted unparsable Dart (`\${...}` instead of `${...}`)**: the
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
