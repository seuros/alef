//! `is_dependency_error` must separate "the artifact was never built" from "the generated code
//! does not compile".
//!
//! ~keep A `Fail` whose message `is_dependency_error` accepts is rewritten by
//! `runner::finalize_result` into `Unavailable` + `unresolved_dependency`, captioned "run `alef
//! build` first". That reclassification is the only thing standing between a codegen defect and
//! a green run: `Unavailable` is tallied apart from `failed`, so a snippet that does not compile
//! is reported as an environment gap and the run does not go red. Task #215 shipped exactly that
//! — 283 generated Rust snippets referencing an unbound `result` (`E0425`) and 51 generated Java
//! snippets calling a method the record does not declare (`cannot find symbol`) were all counted
//! `unavailable`, so nothing failed.
//!
//! The rule these tests pin is the one `typescript.rs` already adopted for task #130: a pattern
//! is a dependency pattern only when it can ONLY mean "the name came from outside this snippet
//! and could not be resolved at all". An unresolved *local* name, an absent member on a type
//! that did resolve, a type mismatch, or a bare "could not compile" summary are all defects in
//! the snippet, and when the output mixes the two the answer is "not a dependency error" — fail
//! loudly rather than shrug.

use super::SnippetValidator;
use super::csharp::CsharpValidator;
use super::go::GoValidator;
use super::java::JavaValidator;
use super::rust::RustValidator;
use super::swift::SwiftValidator;

/// The compiler output alef itself generated in a consumer repo at 0.67.2: the snippet's call
/// was emitted as `let _ = convert(...)` while the presentation layer emitted
/// `println!("{:?}", result.content)`. Every one of the repo's 283 Rust snippets carried it, and
/// every one was reported `unavailable`. ~keep
const RUST_UNBOUND_RESULT: &str = "\
error[E0425]: cannot find value `result` in this scope
 --> src/main.rs:5:22
  |
5 |     println!(\"{:?}\", result.content);
  |                      ^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
error: could not compile `snippet` (bin \"snippet\") due to 1 previous error
";

/// The shape that genuinely means "the binding package was never built": the crate itself does
/// not resolve. Narrowing must not swallow this — an operator who has not run `alef build` still
/// needs the build hint rather than a wall of red. ~keep
const RUST_MISSING_CRATE: &str = "\
error[E0432]: unresolved import `sample_bindings`
 --> src/main.rs:1:5
  |
1 | use sample_bindings::convert;
  |     ^^^^^^^^^^^^^^^^ use of unresolved module or unlinked crate `sample_bindings`

error: aborting due to 1 previous error
error: could not compile `snippet` (bin \"snippet\") due to 1 previous error
";

/// The consumer Java shape: `BatchObject` resolved fine, it simply has no `error()` accessor. ~keep
const JAVA_MISSING_MEMBER: &str = "\
Example.java:12: error: cannot find symbol
        System.out.println(result.error().statusCode());
                                 ^
  symbol:   method error()
  location: variable result of type BatchObject
1 error
";

const JAVA_MISSING_PACKAGE: &str = "\
Example.java:1: error: package dev.sample.bindings does not exist
import dev.sample.bindings.*;
^
1 error
";

#[test]
fn rust_unbound_result_is_a_compile_error_not_a_missing_dependency() {
    assert!(
        !RustValidator.is_dependency_error(RUST_UNBOUND_RESULT),
        "E0425 on a name the snippet itself was supposed to bind is a codegen defect, not an \
         unbuilt artifact: {RUST_UNBOUND_RESULT}"
    );
}

#[test]
fn rust_unresolved_crate_import_is_still_a_missing_dependency() {
    assert!(
        RustValidator.is_dependency_error(RUST_MISSING_CRATE),
        "E0432 against the binding crate is the genuine `alef build` case: {RUST_MISSING_CRATE}"
    );
}

/// A type mismatch reaches `rustc` only once every name resolved, so it can never be an unbuilt
/// artifact. `E0308` was on the accepted list, which alone made every mistyped generated snippet
/// report as `unavailable`. ~keep
#[test]
fn rust_type_mismatch_is_not_a_missing_dependency() {
    let output = "\
error[E0308]: mismatched types
 --> src/main.rs:3:26
  |
3 |     let value: String = 1;
  |                ------   ^ expected `String`, found integer

error: aborting due to 1 previous error
";
    assert!(
        !RustValidator.is_dependency_error(output),
        "E0308 is a type error, not a missing dependency: {output}"
    );
}

/// Mirrors `typescript::is_dependency_error_declines_a_mixed_batch`: one real unresolved import
/// alongside one genuine defect is not confidently an environment gap, so the whole result stays
/// `Fail` with the compiler's own text. ~keep
#[test]
fn rust_mixed_output_is_not_a_missing_dependency() {
    let output = format!("{RUST_MISSING_CRATE}{RUST_UNBOUND_RESULT}");
    assert!(
        !RustValidator.is_dependency_error(&output),
        "a run mixing a genuine defect with an unresolved import must not be relabeled: {output}"
    );
}

#[test]
fn java_missing_member_is_a_compile_error_not_a_missing_dependency() {
    assert!(
        !JavaValidator.is_dependency_error(JAVA_MISSING_MEMBER),
        "`cannot find symbol` for a method on a type that resolved is a codegen defect: {JAVA_MISSING_MEMBER}"
    );
}

#[test]
fn java_missing_package_is_still_a_missing_dependency() {
    assert!(
        JavaValidator.is_dependency_error(JAVA_MISSING_PACKAGE),
        "`package ... does not exist` is the genuine unbuilt-artifact shape: {JAVA_MISSING_PACKAGE}"
    );
}

/// Go's `undefined: x` fires for an unexported local just as readily as for a package the module
/// never provided, so it is the same ambiguous shape task #130 rejected for `TS2304`. ~keep
#[test]
fn go_undefined_local_is_not_a_missing_dependency() {
    let output = "./main.go:9:14: undefined: result\n";
    assert!(
        !GoValidator.is_dependency_error(output),
        "`undefined:` alone cannot distinguish a defect from an unbuilt package: {output}"
    );
}

#[test]
fn go_missing_module_is_still_a_missing_dependency() {
    let output = "main.go:4:2: no required module provides package example.com/binding; to add it:\n";
    assert!(
        GoValidator.is_dependency_error(output),
        "an unprovided module is the genuine dependency shape: {output}"
    );
}

/// Swift's `cannot find 'x' in scope` is the direct analogue of `TS2304` / `E0425`. ~keep
#[test]
fn swift_unresolved_name_is_not_a_missing_dependency() {
    let output = "snippet.swift:7:13: error: cannot find 'result' in scope\n";
    assert!(
        !SwiftValidator::default().is_dependency_error(output),
        "`cannot find ... in scope` is ambiguous and must not be relabeled: {output}"
    );
}

#[test]
fn swift_missing_module_is_still_a_missing_dependency() {
    let output = "snippet.swift:1:8: error: no such module 'SampleBindings'\n";
    assert!(
        SwiftValidator::default().is_dependency_error(output),
        "`no such module` is the genuine unbuilt-artifact shape: {output}"
    );
}

/// `CS0103` ("The name 'x' does not exist in the current context") is C#'s ambiguous unresolved
/// name, exactly like `TS2304`. `CS0246`/`CS0234` name a type or namespace the compiler could not
/// locate at all, which is the real unbuilt-package shape. ~keep
#[test]
fn csharp_unresolved_name_is_not_a_missing_dependency() {
    let output = "Program.cs(9,13): error CS0103: The name 'result' does not exist in the current context\n";
    assert!(
        !CsharpValidator.is_dependency_error(output),
        "CS0103 is ambiguous and must not be relabeled: {output}"
    );
}

#[test]
fn csharp_missing_namespace_is_still_a_missing_dependency() {
    let output = "Program.cs(1,7): error CS0246: The type or namespace name 'SampleBindings' could not be found\n";
    assert!(
        CsharpValidator.is_dependency_error(output),
        "CS0246 is the genuine unbuilt-package shape: {output}"
    );
}

/// A validator that answered `true` for everything would pass every "is not a dependency error"
/// test above only by accident of the assertions' direction; a validator that answered `false`
/// for everything would pass them all. Both directions are asserted for every language above,
/// so neither degenerate answer survives — this test pins that pairing so a future narrowing
/// cannot delete the positive half and leave a vacuous suite. ~keep
#[test]
fn every_validator_answers_both_directions() {
    let cases: Vec<(&str, bool, bool)> = vec![
        (
            "rust",
            RustValidator.is_dependency_error(RUST_MISSING_CRATE),
            RustValidator.is_dependency_error(RUST_UNBOUND_RESULT),
        ),
        (
            "java",
            JavaValidator.is_dependency_error(JAVA_MISSING_PACKAGE),
            JavaValidator.is_dependency_error(JAVA_MISSING_MEMBER),
        ),
    ];
    for (language, positive, negative) in cases {
        assert!(positive, "{language}: the genuine dependency shape must classify");
        assert!(!negative, "{language}: the compile-error shape must not classify");
    }
}
