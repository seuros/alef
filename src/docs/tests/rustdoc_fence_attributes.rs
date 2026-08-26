//! Regression: rustdoc fence attributes must not be copied verbatim into generated
//! markdown.
//!
//! A rustdoc example fence carries rustdoc's own comma-separated test attributes
//! (` ```rust,no_run `). Markdown renderers and `alef snippets audit --docs` read the
//! whole first whitespace-delimited token as the language, so `rust,no_run` lands as an
//! unknown fence language in the generated page. The attributes are meaningful only to
//! rustdoc's doctest harness and carry nothing a rendered page can use, so they must be
//! dropped while the language tag survives.
//!
//! Dropping them upstream is the only available fix: the doc comment cannot omit
//! `no_run` without making the doctest actually execute. ~keep

use crate::core::config::Language;
use crate::core::ir::TypeRef;
use crate::docs::generate_docs;
use crate::docs::test_helpers::{make_function, make_minimal_api, make_param, make_test_config};
use crate::snippets::audit::{AuditConfig, AuditIssueKind, audit};

use super::empty_type;

/// Generate `api-rust.md` for a function whose `# Example` fence carries `fence_info`.
fn rust_page_with_example_fence(fence_info: &str) -> String {
    let mut api = make_minimal_api("1.0.0");
    let mut function = make_function(
        "count_units",
        vec![make_param("text", TypeRef::String, false)],
        TypeRef::Named("UnitCount".to_string()),
        false,
        Some("UnitError"),
    );
    function.doc = format!(
        "Count the units in a string.\n\n# Example\n\n```{fence_info}\n\
         let total = count_units(\"hello\");\n```"
    );
    api.functions = vec![function];
    api.types = vec![empty_type("UnitCount")];

    let files = generate_docs(&api, &make_test_config(), &[Language::Rust], "docs").expect("docs generate");
    files
        .iter()
        .find(|file| file.path.to_string_lossy().contains("api-rust"))
        .expect("api-rust page is generated")
        .content
        .clone()
}

/// Generate `api-python.md` for a function whose doc body carries a Rust code fence
/// (`fence_info`) *outside* the `# Example`/`# Examples` header. `strip_rust_sections`
/// strips those headers unconditionally regardless of fence content, so a fence there
/// can never exercise the Rust-code-block detector this test targets -- only a
/// body-level fence, on a non-Rust target language, walks the path that leaked raw
/// rustdoc attribute fences (`is_rust_code_block` in `doc_cleaning.rs`) into 15
/// generated reference pages in task #370. ~keep
fn python_page_with_body_fence(fence_info: &str) -> String {
    let mut api = make_minimal_api("1.0.0");
    let mut function = make_function(
        "count_units",
        vec![make_param("text", TypeRef::String, false)],
        TypeRef::Named("UnitCount".to_string()),
        false,
        Some("UnitError"),
    );
    function.doc = format!(
        "Count the units in a string.\n\n```{fence_info}\nuse mylib::count_units;\n\
         let total = count_units(\"hello\").unwrap();\n```"
    );
    api.functions = vec![function];
    api.types = vec![empty_type("UnitCount")];

    let files = generate_docs(&api, &make_test_config(), &[Language::Python], "docs").expect("docs generate");
    files
        .iter()
        .find(|file| file.path.to_string_lossy().contains("api-python"))
        .expect("api-python page is generated")
        .content
        .clone()
}

/// Every opening-fence info string in `page`, in emission order.
fn opening_fence_infos(page: &str) -> Vec<String> {
    let mut infos = Vec::new();
    let mut open = false;
    for line in page.lines() {
        let Some(rest) = line.trim().strip_prefix("```") else {
            continue;
        };
        if open {
            open = false;
            continue;
        }
        open = true;
        infos.push(rest.trim().to_string());
    }
    infos
}

/// Run the real `alef snippets audit --docs` fence check over `page`.
fn audit_unknown_language_messages(page: &str) -> Vec<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("api-rust.md"), page).expect("write page");
    let report = audit(&AuditConfig {
        docs_dirs: vec![dir.path().to_path_buf()],
        snippet_dirs: Vec::new(),
        require_frontmatter: false,
        include_base_paths: Vec::new(),
        configured_references: Vec::new(),
        exclude: Vec::new(),
        ..AuditConfig::default()
    });
    report
        .issues
        .into_iter()
        .filter(|issue| issue.kind == AuditIssueKind::UnknownLanguage)
        .map(|issue| issue.message)
        .collect()
}

#[test]
fn rustdoc_test_attribute_is_dropped_from_the_generated_fence() {
    let page = rust_page_with_example_fence("rust,no_run");
    assert!(
        !page.contains("```rust,no_run"),
        "the rustdoc-only `no_run` attribute must not reach the generated page:\n{page}"
    );
    assert!(
        opening_fence_infos(&page).iter().any(|info| info == "rust"),
        "the `rust` language tag must survive the attribute strip; fences: {:?}",
        opening_fence_infos(&page)
    );
}

#[test]
fn generated_page_passes_the_docs_fence_audit() {
    let page = rust_page_with_example_fence("rust,no_run");
    let findings = audit_unknown_language_messages(&page);
    assert!(
        findings.is_empty(),
        "`alef snippets audit --docs` must report no unknown fence language; got {findings:?}\n{page}"
    );
}

/// Multiple attributes, and an attribute-only fence (rustdoc treats a bare `no_run` as
/// Rust) must both reduce to a plain `rust` tag. ~keep
#[test]
fn multiple_and_bare_rustdoc_attributes_reduce_to_the_language_tag() {
    for fence_info in ["rust,no_run,should_panic", "rust,edition2021", "no_run", "ignore"] {
        let page = rust_page_with_example_fence(fence_info);
        assert!(
            audit_unknown_language_messages(&page).is_empty(),
            "fence info `{fence_info}` must audit clean; page:\n{page}"
        );
    }
}

/// task #370: every prior test in this file only ever generates and audits
/// `api-rust.md`, with the fence under `# Example` -- the one combination where
/// `examples::authored_example_block` already rewrote the fence tag to a bare `rust`
/// before the audit ever ran, and the header-stripped section hid the raw fence from
/// `is_rust_code_block` entirely. A body-level fence on a *non*-Rust page walks the
/// code path that actually shipped broken (`doc_cleaning::is_rust_code_block`'s
/// exact-string allowlist), so this is the case that must be asserted through the real
/// audit for the fix to mean anything. ~keep
#[test]
fn body_level_rustdoc_attribute_fence_is_stripped_from_a_non_rust_page() {
    for fence_info in [
        "rust",
        "no_run",
        "ignore",
        "should_panic",
        "compile_fail",
        "rust,no_run",
        "rust,ignore",
        "rust,no_run,should_panic",
        "rust,edition2021",
    ] {
        let page = python_page_with_body_fence(fence_info);
        assert!(
            audit_unknown_language_messages(&page).is_empty(),
            "fence info `{fence_info}` must audit clean on a non-Rust page; page:\n{page}"
        );
        assert!(
            !page.contains("use mylib::count_units"),
            "the Rust-only body fence for `{fence_info}` must be stripped from the Python page, not leaked verbatim:\n{page}"
        );
    }
}

/// Control: the audit CAN report an unknown fence language, so the checks above are not
/// vacuously green on a helper that never finds anything.
///
/// This used to assert on `rust,no_run` itself -- which was the pre-fix bug, not a
/// control. Once `Language::from_fence_info` correctly recognizes `rust,no_run` as
/// Rust, that fixture stops being "genuinely unknown" and the assertion would have
/// silently started proving nothing while still reading green. Use a fence tag with no
/// relationship to the rustdoc attribute vocabulary so the control stays independent of
/// the fix it is guarding. ~keep
///
/// task: a bare unrecognized single-token tag (the original `some_unknown_language` this
/// test used) no longer flags at all -- a human-authored docs page may legitimately fence a
/// language alef does not generate bindings for (`astro`, `mdx`, `hcl`, ...), and that is
/// prose decoration, not a defect (see `snippets::audit::tag_claims_a_binding_target_language`).
/// The control now uses a fence that genuinely still must fail: it combines a real
/// binding-target language (`rust`) with a token that is not a recognized rustdoc doctest
/// attribute either, so the combined info string still resolves to `Unknown` while
/// unambiguously claiming a real target language. ~keep
#[test]
fn the_docs_fence_audit_still_reports_a_genuinely_unknown_language() {
    let page = "# Page\n\n```rust,definitely_bogus\nlet x = 1;\n```\n";
    let findings = audit_unknown_language_messages(page);
    assert_eq!(
        findings,
        vec!["unknown fenced code language: rust,definitely_bogus".to_string()],
        "the audit must flag a fence that claims a real binding-target language and gets it \
         wrong — otherwise the assertions above prove nothing"
    );
}
