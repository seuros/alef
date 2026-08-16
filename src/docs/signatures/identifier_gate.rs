use super::*;

// ---------------------------------------------------------------------------
// ~keep Synthetic reproduction of the tslp `DownloadManager` constructor audit (measured
// against a real `javac` run, not inference -- see formatting.rs's `reserved_words` doc
// comment). liter-llm does not exercise `[crates.constructors]` -- no such function exists
// in the canary -- so this cannot be verified against liter-llm's own generated output;
// per the coordinator, a repo that doesn't exercise a code path cannot clear it. These
// tests build the exact colliding shape by hand: a static method named `new` on an opaque
// type, which is what a curated constructor renders as today (see the "not yet modeled"
// finding for item 1 -- the docs pipeline has no awareness of `ClientConstructorConfig`).
// ---------------------------------------------------------------------------

fn download_manager_new_method() -> crate::core::ir::MethodDef {
    make_method(
        "new",
        vec![make_param("version", TypeRef::String, false)],
        TypeRef::Named("DownloadManager".to_string()),
        false,
        true,
        None,
    )
}

#[test]
#[should_panic(expected = "reserved word")]
fn test_java_constructor_named_new_is_rejected_by_the_identifier_gate() {
    render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Java,
        TEST_PREFIX,
    );
}

#[test]
#[should_panic(expected = "reserved word")]
fn test_dart_constructor_named_new_is_rejected_by_the_identifier_gate() {
    render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Dart,
        TEST_PREFIX,
    );
}

/// ~keep Two defects, not one: Go's real bug (on the default, unconfigured opaque-
/// constructor path the peer actually measured -- a plain FFI export run through Go's
/// generic name mapping, not `[crates.constructors]`) is shape, not a reserved word --
/// `New` is legal Go. The gate correctly does not fire here. This asserts the fully
/// source-verified real shape: no receiver (`gen_method_wrapper`'s static template) and a
/// pointer-wrapped return (`go_optional_type` pointer-wraps every Named return,
/// unconditionally -- see signatures.rs's `go_return_type`).
#[test]
fn test_go_constructor_named_new_is_not_caught_by_the_identifier_gate() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Go,
        TEST_PREFIX,
    );
    assert_eq!(sig, "func DownloadManagerNew(version string) *DownloadManager");
}

/// ~keep C#'s real defect (a fabricated static factory instead of a real constructor) is a
/// name-*selection* problem: `func_name` PascalCases `new` to `New`, which collides with
/// nothing. The gate cannot and does not catch this class of bug.
#[test]
fn test_csharp_constructor_named_new_is_not_caught_by_the_identifier_gate() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Csharp,
        TEST_PREFIX,
    );
    assert!(sig.contains("New"), "{sig}");
}

/// ~keep Zig's real defect is shape (a method rendering over what the backend actually
/// emits as a free function `new_download_manager`) -- `new` is not reserved in Zig.
#[test]
fn test_zig_constructor_named_new_is_not_caught_by_the_identifier_gate() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert!(sig.contains("new"), "{sig}");
}

/// ~keep Elixir is tslp's free positive control: the one language whose real idiom
/// (`DownloadManager.new/1`) happens to coincide with what the template emits, because
/// Elixir does not reserve `new` either way. The one success and the five failures share
/// the same mechanism -- a template that never checked -- which is the strongest form of
/// the "formula, not lookup" claim this whole round has been building.
#[test]
fn test_elixir_constructor_named_new_is_the_positive_control() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Elixir,
        TEST_PREFIX,
    );
    assert!(sig.contains("new"), "{sig}");
}
