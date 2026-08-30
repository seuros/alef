//! Regression coverage for the Elixir `|>` / `not in` precedence defect in streaming assertions.
//!
//! Elixir's operator table binds `in` / `not in` TIGHTER than `|>`. A streaming accessor that
//! led with a pipe therefore could not be substituted into `render_assertion`'s `not_empty` arm
//! (`assert <expr> not in [nil, "", [], %{}]`): the parser attached the `not in [...]` tail to
//! the pipe's right-hand side and rejected the whole line. Observed in a consumer's E2E elixir
//! job as:
//!
//! ```text
//! cannot pipe chunks into Enum.flat_map(...) not in [nil, "", [], %{}]
//! ```
//!
//! The fix is in `streaming_assertions::accessors`: every Elixir streaming accessor is emitted
//! as a primary expression (a single call), never a bare pipe chain, because call sites paste it
//! into operator contexts they do not parenthesize.
//!
//! Lives in its own file rather than growing `elixir/assertions.rs`, which is over the repo's
//! 1,000-line cap and may not grow (see `file-modularization` in CLAUDE.md). ~keep

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use super::snippet::render_snippet_body;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::streaming_assertions::{STREAMING_VIRTUAL_FIELDS, StreamingFieldResolver};
use crate::e2e::config::{E2eConfig, StreamingConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

/// The exact `tool_calls` accessor the Elixir backend must emit.
const TOOL_CALLS_ACCESSOR: &str =
    "Enum.flat_map(chunks, fn c -> (Map.get((List.first(c.choices) || %{}).delta, :tool_calls, []) || []) end)";

/// The exact `stream_content` accessor the Elixir backend must emit.
const STREAM_CONTENT_ACCESSOR: &str = "Enum.join(Enum.map(chunks, fn c -> \
     Map.get(Map.get((Enum.at(c.choices, 0) || %{}), :delta, %{}), :content, \"\") end), \"\")";

/// The pre-fix `tool_calls` accessor, kept verbatim so the scanner below can be shown to reject
/// the shape that actually broke a consumer build rather than passing vacuously. ~keep
const DEFECTIVE_TOOL_CALLS_ACCESSOR: &str =
    "chunks |> Enum.flat_map(fn c -> ((List.first(c.choices) || %{}).delta |> Map.get(:tool_calls, [])) || [] end)";

/// Lower bound on how many Elixir streaming accessors the defect-class sweep must inspect: the
/// 13 entries of `STREAMING_VIRTUAL_FIELDS` plus `usage` and three `tool_calls` deep paths.
const MIN_SWEPT_ACCESSORS: usize = 17;

/// True when `expr` contains a `|>` outside every bracket group — the shape that cannot be
/// pasted in front of `not in [...]`. Pipes nested inside `(...)`, `[...]` or `%{...}` are
/// already delimited and parse correctly. String literals are skipped so a `"|>"` inside a
/// generated Elixir string is not mistaken for an operator. ~keep
fn has_top_level_pipe(expr: &str) -> bool {
    let bytes = expr.as_bytes();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            match byte {
                b'\\' => index += 1,
                b'"' => in_string = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'|' if depth == 0 && bytes.get(index + 1) == Some(&b'>') => return true,
            _ => {}
        }
        index += 1;
    }
    false
}

fn streaming_assertion(assertion_type: &str, field: &str) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

/// Renders one assertion the way `test_case.rs` renders a streaming fixture: the collected list
/// is bound to `chunks`, so the accessor is built over `chunks`, exactly as in the failing job.
fn render_streaming_assertion(assertion_type: &str, field: &str) -> String {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &streaming_assertion(assertion_type, field),
        "chunks",
        &resolver,
        "Sample",
        &HashSet::new(),
        &HashMap::new(),
        false,
        true,
        false,
        false,
    );
    out
}

/// Pins the accessor character-for-character. Restoring the pre-fix
/// `chunks |> Enum.flat_map(fn c -> ... end)` fails on the leading `chunks |> `.
#[test]
fn elixir_tool_calls_accessor_is_a_plain_call_not_a_pipe_chain() {
    let expr = StreamingFieldResolver::accessor("tool_calls", "elixir", "chunks").expect("elixir tool_calls accessor");
    assert_eq!(expr, TOOL_CALLS_ACCESSOR, "got: {expr}");
}

/// Same pin for `stream_content`. Restoring
/// `chunks |> Enum.map(...) |> Enum.join("")` fails on the leading `chunks |> `.
#[test]
fn elixir_stream_content_accessor_is_a_plain_call_not_a_pipe_chain() {
    let expr =
        StreamingFieldResolver::accessor("stream_content", "elixir", "chunks").expect("elixir stream_content accessor");
    assert_eq!(expr, STREAM_CONTENT_ACCESSOR, "got: {expr}");
}

/// The line that actually failed to compile in the consumer's E2E elixir job, pinned whole.
/// Reverting the accessor renders
/// `assert chunks |> Enum.flat_map(...) not in [nil, "", [], %{}]`, which Elixir rejects with
/// "cannot pipe chunks into Enum.flat_map(...) not in [nil, \"\", [], %{}]".
#[test]
fn not_empty_on_tool_calls_emits_a_parsable_membership_test() {
    let out = render_streaming_assertion("not_empty", "tool_calls");
    assert_eq!(
        out,
        format!("      assert {TOOL_CALLS_ACCESSOR} not in [nil, \"\", [], %{{}}]\n"),
        "got: {out}"
    );
}

/// The same latent break on the other pipe-headed accessor: `not_empty` on `stream_content` was
/// never exercised by the consumer, but produced the identical unparsable shape.
#[test]
fn not_empty_on_stream_content_emits_a_parsable_membership_test() {
    let out = render_streaming_assertion("not_empty", "stream_content");
    assert_eq!(
        out,
        format!("      assert {STREAM_CONTENT_ACCESSOR} not in [nil, \"\", [], %{{}}]\n"),
        "got: {out}"
    );
}

/// Defect-class sweep: NO Elixir streaming accessor may lead with a pipe, not just the two that
/// did. A future accessor written as `chunks |> ...` fails here even if nothing asserts
/// `not_empty` on it yet.
#[test]
fn no_elixir_streaming_accessor_leads_with_a_top_level_pipe() {
    let mut fields: Vec<&str> = STREAMING_VIRTUAL_FIELDS.to_vec();
    fields.extend([
        "usage",
        "tool_calls[0]",
        "tool_calls[0].id",
        "tool_calls[0].function.name",
    ]);

    let resolved: Vec<(&str, String)> = fields
        .iter()
        .filter_map(|field| {
            StreamingFieldResolver::accessor_with_streaming_context(
                field,
                "elixir",
                "chunks",
                None,
                Some("StreamEvent"),
            )
            .map(|expr| (*field, expr))
        })
        .collect();

    // ~keep Read the count, not the pass: an accessor that silently resolved to `None` would be
    // swept over without ever being inspected, and the loop below would still report green.
    let unresolved: Vec<&&str> = fields
        .iter()
        .filter(|field| !resolved.iter().any(|(name, _)| name == *field))
        .collect();
    assert!(
        unresolved.is_empty(),
        "elixir accessors resolved to None: {unresolved:?}"
    );
    assert!(
        resolved.len() >= MIN_SWEPT_ACCESSORS,
        "sweep shrank to {} accessors",
        resolved.len()
    );

    for (field, expr) in &resolved {
        assert!(
            !has_top_level_pipe(expr),
            "elixir accessor for '{field}' leads with a pipe and cannot be pasted before \
             `not in [...]`: {expr}"
        );
    }
}

/// Proves the sweep above is wired to the real defect instead of passing on everything: the
/// verbatim pre-fix accessor must be flagged, and the shipped one must not. ~keep
#[test]
fn the_top_level_pipe_scanner_flags_the_shape_that_broke_the_build() {
    assert!(
        has_top_level_pipe(DEFECTIVE_TOOL_CALLS_ACCESSOR),
        "scanner missed the pre-fix accessor"
    );
    assert!(!has_top_level_pipe(TOOL_CALLS_ACCESSOR));
    assert!(!has_top_level_pipe(STREAM_CONTENT_ACCESSOR));
    // A pipe nested inside a bracket group is delimited and legal — the scanner must not
    // report it, or "no top-level pipe" would degrade into "no pipe anywhere". ~keep
    assert!(!has_top_level_pipe("Enum.join(a |> Enum.map(f), \"\")"));
    assert!(!has_top_level_pipe("Enum.map(c, fn x -> x |> to_string() end)"));
}

/// Control: a pipe in STATEMENT position is valid Elixir and must survive untouched. A "fix"
/// that removed pipes from the Elixir emitters wholesale would pass every test above and fail
/// this one.
#[test]
fn statement_position_pipe_in_the_streaming_snippet_is_unchanged() {
    let fixture = Fixture {
        id: "sample_stream".into(),
        description: "Sample stream".into(),
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "stream_items".into();
    e2e.call.module = "sample".into();
    e2e.call.result_var = "stream_result".into();
    e2e.call.streaming = Some(StreamingConfig::Enabled(true));

    let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

    assert!(
        body.contains("stream_result = Sample.stream_items() |> Enum.to_list()"),
        "statement-position pipe must be preserved verbatim, got:\n{body}"
    );
}
