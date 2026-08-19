//! The one shared decision point for whether a fixture's declared `error` assertion value can
//! be substantiated by a backend's generated binding.
//!
//! ~keep [`super::declared_error_value`]'s own doc comment documents that fixture authors spell
//! a declared `error` value one of two ways: a message substring (config-validation fixtures,
//! e.g. `"size"`) or an error VARIANT NAME (API-error fixtures, e.g. `"Authentication"`,
//! matching `ErrorVariant.name` in the IR verbatim). The message-or-type-name disjunction every
//! backend renders is correct for the first convention. For the second it is, for most
//! backends, structurally unsatisfiable: the message is lowercase `#[error("...")]` prose that
//! never contains the variant's PascalCase identifier, and the "type name" side is a generic
//! exception/error class the binding never differentiates per variant. Measured across two
//! consumer repos this was the single largest class of e2e failures alef generates: Go
//! 47/162, Java 47/162, C# 45/162, PHP 47/162, Ruby 45/47, C 44/162, Zig 45, Dart 47/127 —
//! every one of them an assertion that could never pass, generated and then run anyway.
//!
//! Call sites across `csharp.rs`, `java/test_method.rs`, `go/test_function.rs`,
//! `zig/test_file.rs`, `dart/test_case.rs`, `ruby/examples.rs`, `c/test_function.rs`,
//! `php/test_method.rs`, `swift/test_method.rs`, `gleam/test_case.rs`, `elixir/test_case.rs`
//! and `r/test_case.rs` used to each hand-roll the same message-or-type-name disjunction with
//! no shared place to record which of them could ever pass. [`classify`] is that place: the
//! only function that decides substantiability, so a verdict change (a backend gains real
//! per-variant identity, or a new backend is added) is a one-place edit instead of another
//! `push_str(&format!(...))` copy of the judgement call.
//!
//! `brew`/`homebrew` deliberately keep their own single combined-output `grep -F` check
//! (`brew/category.rs::render_error_test_body`) rather than routing through here: that
//! generator's own doc comment documents a considered, different rationale — a CLI's error text
//! is the only observable signal it has, and its author's claim is that a variant's type name is
//! typically echoed into that text by the CLI's own error formatting. Nothing in this fix's
//! evidence contradicts that, and brew was not among the measured-broken backends, so it is left
//! untouched rather than reclassified on inference. `wasm` has no `declared_error_value` call
//! site at all and needs no change either.
//!
//! Deliberately NOT fuzzy: [`classify`] never lowercases, splits camelCase, or does a
//! case-insensitive substring compare to try to make the type-name side "work". That would make
//! `"Authentication"` match a message containing `"authentication"` by accident while still
//! never matching `"BadRequest"` against `"bad request"` — trading one precise, honestly-failing
//! assertion for one that passes some of the time for the wrong reason. Where a backend cannot
//! substantiate a variant, the correct output is a registered skip, not a weakened comparison.

use crate::core::ir::{ErrorDef, ErrorVariant};
use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
use crate::e2e::fixture::Fixture;

/// What a backend's error-assertion renderer should do with a fixture's declared `error` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclaredErrorAssertion<'a> {
    /// No `error` assertion declared a value. Callers render their pre-existing bare check
    /// unchanged.
    Undeclared,
    /// Render the value normally: it is either a message-style value (which the
    /// message-or-type-name disjunction already serves) or it names a real error variant this
    /// backend's binding *does* differentiate.
    Assert(&'a str),
    /// The value names a real error variant this backend's generated binding cannot
    /// differentiate from any other variant of the same error type. Callers render
    /// [`skip_line`] instead of an assertion that can never pass.
    Unsubstantiable(&'a str),
}

/// Decide [`DeclaredErrorAssertion`] for `fixture` in `lang`, using `errors`
/// (`ApiSurface::errors`, the crate's full error-type registry) to tell a fixture's
/// variant-name value apart from a message-style one.
///
/// A value that does not match any known `ErrorVariant::name` is always message-style — the
/// exact IR-membership test is what keeps this from ever needing a fuzzy heuristic.
pub(crate) fn classify<'a>(lang: &str, fixture: &'a Fixture, errors: &[ErrorDef]) -> DeclaredErrorAssertion<'a> {
    let Some(declared) = super::declared_error_value(fixture) else {
        return DeclaredErrorAssertion::Undeclared;
    };
    let named_variant = errors
        .iter()
        .flat_map(|error| &error.variants)
        .find(|variant| variant.name == declared);
    match named_variant {
        None => DeclaredErrorAssertion::Assert(declared),
        Some(variant) if substantiates_variant_identity(lang, variant) => DeclaredErrorAssertion::Assert(declared),
        Some(_) => DeclaredErrorAssertion::Unsubstantiable(declared),
    }
}

/// Whether `lang`'s generated binding gives the thrown/returned error a way to differ, per
/// variant, from another value of the SAME error type — grounded in what that backend's own
/// codegen emits today, not a hypothetical improvement. Citations are the investigation's,
/// condensed here so the verdict and its evidence stay next to each other.
///
/// - `python`: `pyo3::create_exception!` gives every variant its own exception class, name
///   -derived, unconditionally (`backends/pyo3/gen_bindings/errors.rs::gen_exceptions_py`,
///   `codegen/error_gen/shared.rs::python_exception_name`). Always substantiable.
/// - `go`, `java`, `zig`: sit on the C ABI `#[alef(error_code = N)]` taxonomy
///   (`backends/ffi/gen_bindings/helpers.rs::gen_last_error`) and dispatch to a variant-named
///   error/exception/error-set-member only for variants that declared a code; an uncoded variant
///   collapses to a generic wrapper (Go: `*errors.errorString`; Java: the flat infra exception;
///   Zig: `error.UnknownFfiError`). Conditional on `variant.error_code`.
/// - `c`: the C ABI exposes a variant's code as `{prefix}_last_error_code()`, never as a string
///   (`e2e/codegen/c/test_function.rs`'s own doc comment on `emit_c_error_epilogue`), and the
///   generated test only ever compares message text — so even a coded variant cannot be named
///   today. Never substantiable.
/// - `dart`: flutter_rust_bridge 2.x — the third-party generator alef's dart backend drives, not
///   alef's own code — decodes every Rust error as a raw `String`
///   (`e2e/codegen/dart/test_case.rs`'s own doc comment). Never substantiable.
/// - `csharp`: `GetLastError()` dispatches to a `{Variant}Exception` by matching the live
///   message's literal prefix against `ErrorVariant.message_template`
///   (`backends/csharp/gen_bindings/methods/class.rs`), but opaque-handle and null-result throw
///   sites bypass it and always throw the flat base exception. The e2e generator has no way to
///   know, per fixture, which throw site a call takes, so treated as never reliably
///   substantiable — alef's own generator code, just not wired everywhere yet.
/// - `php`: exactly one exception class exists for the whole extension
///   (`backends/php/gen_bindings/type_stubs.rs`, `class_name = extension_name.to_pascal_case()`),
///   even though ext-php-rs supports defining as many PHP exception classes as alef wants.
///   Never substantiable today.
/// - `swift`: swift-bridge generates a full per-variant `enum`
///   (`backends/swift/gen_bindings/errors.rs`), but no real business-call failure ever
///   constructs a case of it — real throw sites use generic `ServiceError` cases or a
///   `Result<T, String>` shim instead. The capability exists and is simply unused.
/// - `ruby`: every fallible call throws a fixed `RuntimeError` via
///   `magnus::Error::new(ruby.exception_runtime_error(), e.to_string())`; no per-variant class is
///   ever defined, even though Magnus supports custom exception classes the same way Python's
///   `create_exception!` does.
/// - `elixir`, `gleam`: the Rustler NIF boundary both ride on collapses every error to a string
///   via `.map_err(|e| e.to_string())`; a dedicated error-converter exists in the generator but is
///   dead code that also stringifies. Gleam's nominally-typed external signature does not change
///   this — it is backed by the same NIF glue.
/// - `r`: no error-conversion code exists in the R backend at all; extendr's default panic
///   handling surfaces a generic `simpleError`.
/// - `typescript` (NAPI): every throw site is `napi::Error::new(Status::GenericFailure,
///   e.to_string())` — generic status, generic `.name`, message only, even though NAPI-RS
///   supports custom JS `Error` subclasses.
///
/// Any language absent from this match keeps today's behaviour (`true`), so a backend this fix
/// did not audit and rewrite is never silently weakened.
fn substantiates_variant_identity(lang: &str, variant: &ErrorVariant) -> bool {
    match lang {
        "python" => true,
        "go" | "java" | "zig" => variant.error_code.is_some(),
        // `node`, not `typescript`: `TypeScriptCodegen::language_name()` returns `"node"` (it
        // covers both the NAPI/node and WASM targets via a shared `lang` parameter) — this is
        // the literal string every `classify(lang, ..)` call site in `typescript/` passes.
        // `"typescript"` here was dead: it never matched what the backend actually threads
        // through, so the wiring silently fell through to the `true` default below. ~keep
        "c" | "dart" | "csharp" | "php" | "swift" | "ruby" | "elixir" | "gleam" | "r" | "node" => false,
        _ => true,
    }
}

/// Whether a backend's `Unsubstantiable` verdict is a real ABI/toolchain property no amount of
/// alef-only work changes, or alef's own generator simply not preserving an identity that
/// backend's runtime could carry. Mirrors `field_skip::SkipClass`'s own
/// `LanguageLimitation`/`GeneratorGap` split, applied to this one axis. ~keep
fn skip_variant_for(lang: &str) -> AssertionTypeSkip {
    match lang {
        // `c`: the ABI has no string identity at all. `dart`: the third-party
        // flutter_rust_bridge decode step, not alef's own codegen, throws away the type.
        // `go`/`java`/`zig`: whether THIS variant carries a stable ABI taxonomy code is a
        // property of the crate's declared error shape, not alef's generator.
        "c" | "dart" | "go" | "java" | "zig" => AssertionTypeSkip::DeclaredErrorVariantNotSubstantiated,
        // Every other audited backend's own runtime supports distinct per-variant error
        // identities (a class, an atom, a condition); alef's generator just does not build one
        // yet. Fixable in a future alef release, not a permanent limit.
        _ => AssertionTypeSkip::DeclaredErrorVariantNotYetPreservedByGenerator,
    }
}

/// The `skipped:` line for a declared `error` value naming a variant this backend cannot
/// substantiate. Records the skip on the shared ledger itself — mirrors
/// `error_path_assertions::render` — so a call site cannot wire the wording in and forget the
/// gate. Indentation and comment syntax stay at the call site, matching
/// `field_skip::nested_wildcard_skip_line`. Carries no trailing newline.
pub(crate) fn skip_line(indent: &str, comment_open: &str, variant: &str, fixture_id: &str, language: &str) -> String {
    let line = format!(
        "{indent}{comment_open} skipped: {}",
        skip_variant_for(language).message(variant)
    );
    super::fail_on_unsupported_assertion_type_markers(&line, language, fixture_id);
    line
}

#[cfg(test)]
mod tests {
    use super::{DeclaredErrorAssertion, classify, skip_line};
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn error_assertion(value: &str) -> Assertion {
        Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        }
    }

    fn fixture_with(value: &str) -> Fixture {
        Fixture {
            id: "declares_error".to_string(),
            assertions: vec![error_assertion(value)],
            ..Fixture::default()
        }
    }

    fn coded_variant(name: &str, code: Option<u32>) -> ErrorVariant {
        ErrorVariant {
            name: name.to_string(),
            error_code: code,
            is_unit: true,
            ..ErrorVariant::default()
        }
    }

    fn error_def(variants: Vec<ErrorVariant>) -> ErrorDef {
        ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants,
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn no_declared_value_is_undeclared() {
        let fixture = Fixture {
            id: "no_error".to_string(),
            ..Fixture::default()
        };
        assert_eq!(classify("php", &fixture, &[]), DeclaredErrorAssertion::Undeclared);
    }

    /// A message-style value (not a known variant name) is always renderable, even against a
    /// backend that can never substantiate a real variant — this is the case that must NOT
    /// regress: config-validation fixtures keep passing exactly as they do today.
    #[test]
    fn message_style_value_is_always_assertable() {
        let fixture = fixture_with("size");
        let errors = vec![error_def(vec![coded_variant("Authentication", None)])];
        assert_eq!(
            classify("php", &fixture, &errors),
            DeclaredErrorAssertion::Assert("size")
        );
        assert_eq!(classify("c", &fixture, &errors), DeclaredErrorAssertion::Assert("size"));
    }

    /// With no `errors` registry supplied at all (most existing unit tests across the fixed
    /// backends construct fixtures this way), a variant-shaped value is indistinguishable from a
    /// message-style one and stays on the `Assert` path — so pre-existing tests that don't thread
    /// real IR data through stay byte-identical. Only tests that explicitly supply an `ErrorDef`
    /// exercise the skip path.
    #[test]
    fn variant_shaped_value_with_no_ir_data_still_asserts() {
        let fixture = fixture_with("Authentication");
        assert_eq!(
            classify("php", &fixture, &[]),
            DeclaredErrorAssertion::Assert("Authentication")
        );
    }

    #[test]
    fn python_always_substantiates_a_known_variant() {
        let fixture = fixture_with("Authentication");
        let errors = vec![error_def(vec![coded_variant("Authentication", None)])];
        assert_eq!(
            classify("python", &fixture, &errors),
            DeclaredErrorAssertion::Assert("Authentication")
        );
    }

    #[test]
    fn php_never_substantiates_a_known_variant() {
        let fixture = fixture_with("Authentication");
        let errors = vec![error_def(vec![coded_variant("Authentication", Some(100))])];
        assert_eq!(
            classify("php", &fixture, &errors),
            DeclaredErrorAssertion::Unsubstantiable("Authentication")
        );
    }

    #[test]
    fn c_never_substantiates_a_known_variant_even_when_coded() {
        let fixture = fixture_with("Authentication");
        let errors = vec![error_def(vec![coded_variant("Authentication", Some(100))])];
        assert_eq!(
            classify("c", &fixture, &errors),
            DeclaredErrorAssertion::Unsubstantiable("Authentication")
        );
    }

    /// Go/Java/Zig are conditional: a coded variant is still assertable, an uncoded one is not.
    #[test]
    fn go_java_zig_are_conditional_on_error_code() {
        let fixture = fixture_with("Authentication");
        let coded = vec![error_def(vec![coded_variant("Authentication", Some(100))])];
        let uncoded = vec![error_def(vec![coded_variant("Authentication", None)])];
        for lang in ["go", "java", "zig"] {
            assert_eq!(
                classify(lang, &fixture, &coded),
                DeclaredErrorAssertion::Assert("Authentication"),
                "{lang} must assert a coded variant"
            );
            assert_eq!(
                classify(lang, &fixture, &uncoded),
                DeclaredErrorAssertion::Unsubstantiable("Authentication"),
                "{lang} must skip an uncoded variant"
            );
        }
    }

    #[test]
    fn dart_swift_ruby_csharp_elixir_gleam_r_node_never_substantiate_a_known_variant() {
        let fixture = fixture_with("BadRequest");
        let errors = vec![error_def(vec![coded_variant("BadRequest", Some(200))])];
        for lang in ["dart", "swift", "ruby", "csharp", "elixir", "gleam", "r", "node"] {
            assert_eq!(
                classify(lang, &fixture, &errors),
                DeclaredErrorAssertion::Unsubstantiable("BadRequest"),
                "{lang} must skip a known variant it cannot substantiate"
            );
        }
    }

    #[test]
    fn an_unaudited_language_keeps_asserting() {
        let fixture = fixture_with("Authentication");
        let errors = vec![error_def(vec![coded_variant("Authentication", None)])];
        assert_eq!(
            classify("kotlin", &fixture, &errors),
            DeclaredErrorAssertion::Assert("Authentication")
        );
    }

    #[test]
    fn skip_line_renders_the_language_limitation_wording_and_records_it() {
        let _ = crate::e2e::codegen::take_skip_records();
        let line = skip_line("    ", "//", "Authentication", "auth_fails", "c");
        assert_eq!(
            line,
            "    // skipped: declared error variant 'Authentication' not substantiated by this backend's generated \
             error type"
        );
        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "c");
        assert_eq!(records[0].fixture_id, "auth_fails");
        assert_eq!(records[0].field, "Authentication");
    }

    #[test]
    fn skip_line_renders_the_generator_gap_wording_for_a_fixable_backend() {
        let _ = crate::e2e::codegen::take_skip_records();
        let line = skip_line("        ", "#", "BadRequest", "bad_request_fails", "ruby");
        assert_eq!(
            line,
            "        # skipped: declared error variant 'BadRequest' not yet preserved as a distinct identity by \
             this backend's generator"
        );
        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "ruby");
    }
}
