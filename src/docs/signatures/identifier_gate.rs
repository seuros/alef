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

/// ~keep Java's real defect used to be a reserved-word collision: before 28f310259, `new`
/// had no arm in the docs' Java keyword-rename table (only `default` did), so a curated
/// opaque type's default constructor reached `assert_valid_identifier` as the raw word
/// `new` and panicked, aborting the whole docs run. The table is now mirrored from the
/// backend's `safe_java_method_name`, so this shape renders as `create` -- same outcome
/// class as the Go/C#/Zig/Elixir siblings below, not a panic. Asserting `should_panic`
/// here would pin the bug back in.
#[test]
fn test_java_constructor_named_new_is_renamed_to_create_by_the_identifier_gate() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Java,
        TEST_PREFIX,
    );
    assert!(sig.contains("create"), "{sig}");
    assert!(!sig.contains("new"), "{sig}");
}

/// ~keep The property the original (now-removed) `should_panic` test was actually
/// protecting: the identifier gate itself still rejects a raw, un-renamed Java reserved
/// word. `render_method_signature` can no longer exercise this for Java -- `func_name`'s
/// keyword table now renames every `JAVA_KEYWORDS` entry, `new` included, before it can
/// reach the gate (see `test_func_name_java_output_passes_the_identifier_gate` in
/// naming.rs) -- so this calls the gate directly instead of routing through a constructor
/// path that no longer produces an unrenamed keyword.
#[test]
#[should_panic(expected = "reserved word")]
fn test_identifier_gate_still_rejects_a_raw_java_keyword() {
    crate::docs::formatting::assert_valid_identifier("new", Language::Java, "a test context");
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
/// nothing. The gate cannot and does not catch this class of bug, so the docs used to publish
/// `public DownloadManager New(string version)` -- a member `gen_opaque_type` never emits,
/// because `is_static_constructor` diverts this exact shape to
/// `opaque_static_constructor_signature.jinja` (`public {{ class_name }}({{ param_list }})`).
/// Asserting only `contains("New")`, as this test originally did, passed on the wrong output.
#[test]
fn test_csharp_constructor_named_new_is_promoted_to_a_real_constructor() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Csharp,
        TEST_PREFIX,
    );
    assert_eq!(sig, "public DownloadManager(string version)");
}

/// ~keep Zig's real defect is shape (a method rendering over what the backend actually
/// emits as a free function `new_download_manager`) -- `new` is not reserved in Zig, so the
/// gate stays silent and the old `contains("new")` assertion passed on the broken
/// `pub fn new(...)` just as happily as on the correct name.
#[test]
fn test_zig_constructor_named_new_renders_as_a_type_suffixed_free_function() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert_eq!(
        sig,
        "pub fn new_download_manager(version: [:0]const u8) DownloadManager"
    );
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

// ---------------------------------------------------------------------------
// ~keep Cross-checks against the backends themselves, in the shape of
// `test_func_name_java_matches_backend_safe_java_method_name` (naming.rs). The assertions
// above pin a literal string, which stops the *current* divergence from coming back but
// cannot notice the backend moving underneath them; these call the backend's own predicate
// and its own templates, so a rename on either side fails here first.
// ---------------------------------------------------------------------------

/// The docs' `is_csharp_static_constructor` is a hand-copy of the C# backend's
/// `is_static_constructor`. Pin them together over the full clause matrix -- name, staticness,
/// arity and return type each independently flip the answer, and a sampled cross-check would
/// leave three of the four free to drift.
#[test]
fn test_csharp_docs_constructor_predicate_matches_the_backend_predicate() {
    use crate::backends::csharp::gen_bindings::types::constructors::is_static_constructor;

    let string_param = || vec![make_param("version", TypeRef::String, false)];
    let owner = TypeRef::Named("DownloadManager".to_string());
    let cases = vec![
        ("new", string_param(), owner.clone(), true),
        ("new", vec![], owner.clone(), true),
        ("new", string_param(), owner.clone(), false),
        ("new", string_param(), TypeRef::String, true),
        ("new", string_param(), TypeRef::Named("Other".to_string()), true),
        (
            "new",
            string_param(),
            TypeRef::Named("crate::DownloadManager".to_string()),
            true,
        ),
        ("create", string_param(), owner.clone(), true),
        ("with_cache_dir", string_param(), owner, true),
    ];
    for (name, params, return_type, is_static) in cases {
        let method = make_method(name, params, return_type, false, is_static, None);
        assert_eq!(
            is_csharp_static_constructor(&method, "DownloadManager"),
            is_static_constructor(&method, "DownloadManager"),
            "docs and the C# backend disagree about `{name}` (is_static={is_static})"
        );
    }
}

/// The Zig static-method name is a formula (`{method}_{type}`) owned by
/// `opaque_static_signature.jinja`. Render that template with the backend's own inputs and
/// require the documented signature to open with the identifier it produces.
#[test]
fn test_zig_docs_static_name_matches_the_backend_template() {
    let backend = crate::backends::zig::template_env::render(
        "opaque_static_signature.jinja",
        minijinja::context! {
            method_snake => "new",
            type_snake => "download_manager",
            params => "",
            return_ty => "DownloadManager",
        },
    );
    let backend_head = backend
        .split('(')
        .next()
        .expect("template always emits an opening paren");
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert!(
        sig.starts_with(&format!("{backend_head}(")),
        "docs `{sig}` does not open with the backend's `{backend_head}(`"
    );
}

/// Go's static shape is likewise a formula owned by `method_signature_static.jinja`
/// (`func {receiver_type}{method_name}(`). Both the signature *and* the example on the same
/// page must resolve to it -- the page used to print `func DownloadManagerNew(...)` above an
/// example calling `DownloadManager.New(...)`, contradicting itself.
#[test]
fn test_go_docs_signature_and_example_both_match_the_backend_template() {
    let backend = crate::backends::go::template_env::render(
        "method_signature_static.jinja",
        minijinja::context! {
            receiver_type => "DownloadManager",
            method_name => "New",
            params => "",
            return_type => "",
        },
    );
    let backend_head = backend
        .split('(')
        .next()
        .expect("template always emits an opening paren")
        .trim_start_matches("func ")
        .to_string();
    let method = download_manager_new_method();
    let sig = render_method_signature(&method, "DownloadManager", Language::Go, TEST_PREFIX);
    assert!(
        sig.starts_with(&format!("func {backend_head}(")),
        "docs signature `{sig}` does not name the backend's `{backend_head}`"
    );
    let example = crate::docs::examples::render_method_example(&method, "DownloadManager", Language::Go, TEST_PREFIX);
    assert!(
        example.contains(&format!("{backend_head}(\"value\")")),
        "docs example must call the package-level `{backend_head}`: {example}"
    );
    assert!(
        !example.contains("DownloadManager.New("),
        "docs example still uses member syntax Go has no equivalent for: {example}"
    );
}

/// The C# example must construct with `new`, matching the constructor the signature arm now
/// documents. `DownloadManager.New(...)` named a member `gen_opaque_type` never emits.
#[test]
fn test_csharp_docs_example_constructs_with_new() {
    let example = crate::docs::examples::render_method_example(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Csharp,
        TEST_PREFIX,
    );
    assert!(example.contains("new DownloadManager(\"value\")"), "{example}");
    assert!(!example.contains("DownloadManager.New("), "{example}");
}

/// The Zig example must call the same top-level `new_download_manager` the signature names,
/// not `DownloadManager.new(...)` -- Zig has no method syntax for a function the backend emits
/// outside the struct.
#[test]
fn test_zig_docs_example_calls_the_top_level_function() {
    let example = crate::docs::examples::render_method_example(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert!(example.contains("new_download_manager("), "{example}");
    assert!(!example.contains("DownloadManager.new("), "{example}");
}
