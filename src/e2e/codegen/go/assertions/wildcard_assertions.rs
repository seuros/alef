//! Bracket-wildcard ("any element") assertion rendering for the Go e2e generator, plus the
//! non-empty-precondition helper `render_assertion` shares with indexed-element assertions.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::escape::go_string_literal;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::super::json_values::json_to_go;
use super::AssertionRenderContext;

/// Skip an assertion whose field is unavailable on this result type, or dispatch a
/// bracket-wildcard ("any element") traversal assertion to [`render_wildcard_assertion`].
///
/// Both checks must run before `resolve_assertion_target` builds `field_expr`: the
/// wildcard check in particular must run first among the two AND ahead of ordinary field
/// resolution, since that path lowers a wildcard to index 0 and would silently assert on
/// one element only.
///
/// Returns `true` when either check rendered something (a skip marker or the wildcard
/// scan) and the caller must return without falling through; `false` when neither check
/// applied and the caller must proceed to ordinary field resolution.
pub(super) fn render_wildcard_or_unavailable_field(
    out: &mut String,
    assertion: &Assertion,
    context: &AssertionRenderContext<'_>,
) -> bool {
    if context.result_is_simple {
        return false;
    }
    let Some(f) = assertion.field.as_deref() else {
        return false;
    };
    if f.is_empty() {
        return false;
    }
    let field_resolver = context.field_resolver;
    if !field_resolver.is_valid_for_result(f) {
        let _ = writeln!(out, "\t// skipped: {}", FieldSkip::NotAvailableOnResultType.message(f));
        return true;
    }

    // Bracket-wildcard traversal (`links[].link_type`) means "every element", so it must
    // scan the whole slice. Go has no expression-level `any`, so this emits statements
    // rather than folding into `field_expr` below — and it must run before `field_expr`
    // is built, since that path lowers the wildcard to index 0 and would silently assert
    // on one element only. ~keep
    if let Some((array_part, elem_part)) = field_resolver.wildcard_split(f) {
        render_wildcard_assertion(out, assertion, context, f, &array_part, &elem_part);
        return true;
    }
    false
}

/// Emit the `len(arr) == 0` precondition that must precede an `arr[0]` assertion.
///
/// A fixture that asserts on element `0` is stating that the collection has an element;
/// wrapping the assertion in `if len(arr) > 0` instead let an empty result satisfy the
/// whole check, so the test could not fail. `t.Fatalf` stops the test function before the
/// index panics, matching how the generator reports every other unmet precondition.
///
/// The precondition is emitted once per collection per function body: `t.Fatalf` aborts,
/// so repeating it ahead of every indexed assertion would add nothing. The de-duplication
/// compares whole lines, so an identical check emitted at a deeper indentation (inside a
/// nil guard, where it may not run) does not suppress the function-level one.
pub(super) fn emit_non_empty_precondition(out: &mut String, array_expr: &str) {
    let fatal_line = format!(
        "\t\tt.Fatalf(\"expected non-empty %s\", {})",
        go_string_literal(array_expr)
    );
    if out.lines().any(|line| line == fatal_line) {
        return;
    }
    let _ = writeln!(out, "\tif len({array_expr}) == 0 {{");
    let _ = writeln!(out, "{fatal_line}");
    let _ = writeln!(out, "\t}}");
}

/// Per-assertion suffix for Go locals emitted by [`render_wildcard_assertion`].
///
/// Two assertions over the same array would otherwise both declare `found`, which is a
/// redeclaration error in the same function scope. Hashing the assertion's discriminating
/// fields (mirroring the swift backend's `local_suffix`) makes the name unique per
/// assertion while staying stable across regenerations. ~keep
fn wildcard_local_suffix(assertion: &Assertion) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    assertion.assertion_type.hash(&mut hasher);
    assertion.field.hash(&mut hasher);
    assertion
        .value
        .as_ref()
        .map(std::string::ToString::to_string)
        .unwrap_or_default()
        .hash(&mut hasher);
    assertion
        .values
        .as_ref()
        .map(|vs| {
            vs.iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:x}", hasher.finish() & 0xffff_ffff)
}

/// Emit the shared `found := false; for _, e := range <array> { if <cond> { found = true; break } }`
/// scan every wildcard-traversal assertion variant below is built from.
fn emit_wildcard_scan(out: &mut String, array_accessor: &str, local: &str, cond: &str) {
    let _ = writeln!(out, "\t{local} := false");
    let _ = writeln!(out, "\tfor _, e := range {array_accessor} {{");
    let _ = writeln!(out, "\t\tif {cond} {{");
    let _ = writeln!(out, "\t\t\t{local} = true");
    let _ = writeln!(out, "\t\t\tbreak");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}}");
}

/// `contains`/`not_contains` against a single scalar `assertion.value`.
fn render_wildcard_single_value(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    array_accessor: &str,
    elem_accessor: &str,
    suffix: &str,
) {
    let expected = assertion.value.as_ref().expect("guarded by the caller's match arm");
    let go_val = json_to_go(expected);
    let local = format!("found{suffix}");
    let cond = format!("strings.Contains(fmt.Sprintf(\"%v\", {elem_accessor}), {go_val})");
    emit_wildcard_scan(out, array_accessor, &local, &cond);
    if assertion.assertion_type == "contains" {
        let _ = writeln!(out, "\tif !{local} {{");
        let _ = writeln!(
            out,
            "\t\tt.Errorf(\"expected some element of '{field}' to contain %v\", {go_val})"
        );
    } else {
        let _ = writeln!(out, "\tif {local} {{");
        let _ = writeln!(
            out,
            "\t\tt.Errorf(\"expected no element of '{field}' to contain %v\", {go_val})"
        );
    }
    let _ = writeln!(out, "\t}}");
}

/// `contains`/`contains_all`/`not_contains` against a list of `assertion.values`.
fn render_wildcard_multi_value(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    array_accessor: &str,
    elem_accessor: &str,
    suffix: &str,
) {
    let Some(values) = &assertion.values else {
        let _ = writeln!(out, "\t// skipped: '{field}' traversal assertion has no values");
        return;
    };
    let negated = assertion.assertion_type == "not_contains";
    for (i, val) in values.iter().enumerate() {
        let go_val = json_to_go(val);
        let local = format!("found{suffix}v{i}");
        let cond = format!("strings.Contains(fmt.Sprintf(\"%v\", {elem_accessor}), {go_val})");
        emit_wildcard_scan(out, array_accessor, &local, &cond);
        if negated {
            let _ = writeln!(out, "\tif {local} {{");
            let _ = writeln!(
                out,
                "\t\tt.Errorf(\"expected no element of '{field}' to contain %v\", {go_val})"
            );
        } else {
            let _ = writeln!(out, "\tif !{local} {{");
            let _ = writeln!(
                out,
                "\t\tt.Errorf(\"expected some element of '{field}' to contain %v\", {go_val})"
            );
        }
        let _ = writeln!(out, "\t}}");
    }
}

/// `not_empty`: at least one element's rendered value is non-blank.
fn render_wildcard_non_empty(out: &mut String, field: &str, array_accessor: &str, elem_accessor: &str, suffix: &str) {
    let local = format!("found{suffix}");
    let cond = format!("fmt.Sprintf(\"%v\", {elem_accessor}) != \"\"");
    emit_wildcard_scan(out, array_accessor, &local, &cond);
    let _ = writeln!(out, "\tif !{local} {{");
    let _ = writeln!(
        out,
        "\t\tt.Errorf(\"expected some element of '{field}' to be non-empty\")"
    );
    let _ = writeln!(out, "\t}}");
}

/// Emit the statement form of an any-element assertion over a bracket-wildcard path.
pub(super) fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    context: &AssertionRenderContext<'_>,
    field: &str,
    array_part: &str,
    elem_part: &str,
) {
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("\t", "//", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let field_resolver = context.field_resolver;
    let array_accessor = if array_part.is_empty() {
        context.result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "go", context.result_var)
    };
    // `element_accessor`, not `accessor`: the path is already element-relative, so the
    // result-anchoring `accessor` applies would re-prefix it with the container. ~keep
    let elem_accessor = field_resolver.element_accessor(elem_part, "go", "e");
    let suffix = wildcard_local_suffix(assertion);

    match assertion.assertion_type.as_str() {
        "contains" | "not_contains" if assertion.value.is_some() => {
            render_wildcard_single_value(out, assertion, field, &array_accessor, &elem_accessor, &suffix);
        }
        "contains" | "contains_all" | "not_contains" => {
            render_wildcard_multi_value(out, assertion, field, &array_accessor, &elem_accessor, &suffix);
        }
        "not_empty" => {
            render_wildcard_non_empty(out, field, &array_accessor, &elem_accessor, &suffix);
        }
        other => {
            let _ = writeln!(
                out,
                "\t// skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}
