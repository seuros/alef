//! The xUnit attribute must sit on its own line, above the method signature.
//!
//! `test_method.jinja` chooses between `[Fact]` and `[Fact(Skip = "…")]` in a conditional block.
//! Written with whitespace-trimming delimiters (`{%- else %}` / `{%- endif %}`), that block ate
//! the newline after the attribute and emitted `[Fact]    public void Test_X()` — a single line
//! that still compiles, so no test or build caught it, and every generated C# e2e suite carried
//! it. These tests pin the line break itself in both branches.

use super::tests::render_refusal_candidate;
use crate::e2e::fixture::Assertion;

/// Extract the line declaring the test method, plus the line before it.
fn signature_with_preceding_line(rendered: &str) -> (String, String) {
    let lines: Vec<&str> = rendered.lines().collect();
    let index = lines
        .iter()
        .position(|line| line.contains("Test_"))
        .unwrap_or_else(|| panic!("no test method signature in:\n{rendered}"));
    let preceding = index.checked_sub(1).map(|i| lines[i]).unwrap_or_default();
    (preceding.trim().to_string(), lines[index].trim().to_string())
}

#[test]
fn a_plain_fact_attribute_sits_on_its_own_line_above_the_signature() {
    let out = render_refusal_candidate(
        "csharp_fact_layout",
        vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("content".into()),
            ..Assertion::default()
        }],
    );

    let (preceding, signature) = signature_with_preceding_line(&out);
    assert_eq!(preceding, "[Fact]", "the attribute must be its own line, got:\n{out}");
    assert!(
        signature.starts_with("public "),
        "the signature line must carry no attribute, got {signature:?} in:\n{out}"
    );
    assert!(
        !out.contains("[Fact]    public") && !out.contains("[Fact] public"),
        "the attribute and signature must never share a line, got:\n{out}"
    );
}

#[test]
fn a_skipped_fact_attribute_also_sits_on_its_own_line_above_the_signature() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();
    let out = render_refusal_candidate(
        "csharp_fact_skip_layout",
        vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("definitely_missing_field".into()),
            skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
            ..Assertion::default()
        }],
    );
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();

    let (preceding, signature) = signature_with_preceding_line(&out);
    assert!(
        preceding.starts_with("[Fact(Skip = ") && preceding.ends_with(")]"),
        "the skip attribute must be its own complete line, got {preceding:?} in:\n{out}"
    );
    assert!(
        signature.starts_with("public "),
        "the signature line must carry no attribute, got {signature:?} in:\n{out}"
    );
}
