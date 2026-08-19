use super::discarded_call_statement;

/// The defect: applied per line with no guard, the rebinding also matched the allocator
/// teardown every Zig snippet emits, turning `defer _ = gpa.deinit();` into
/// `defer const result = gpa.deinit();`. That is not Zig, and it failed 54 of one consumer's
/// snippets on `expected block or expression`. ~keep
#[test]
fn a_deferred_discard_is_not_a_rebindable_call_statement() {
    assert_eq!(discarded_call_statement("    defer _ = gpa.deinit();"), None);
    assert_eq!(
        discarded_call_statement("    errdefer _ = allocator.free(buffer);"),
        None
    );
}

#[test]
fn a_statement_opening_with_a_discard_yields_the_call_it_discards() {
    assert_eq!(
        discarded_call_statement("    _ = try htmd.convert(allocator, html, null);"),
        Some("try htmd.convert(allocator, html, null);")
    );
    assert_eq!(
        discarded_call_statement("_ = htmd.convert(allocator, html, null);"),
        Some("htmd.convert(allocator, html, null);")
    );
}

/// A discard appearing mid-statement is not a statement-opening discard and must be left alone.
#[test]
fn a_discard_that_does_not_open_the_statement_is_not_matched() {
    assert_eq!(discarded_call_statement("    const pair = .{ _ = 1 };"), None);
}

/// The defect this test pins: every generated visitor callback unconditionally discards its
/// unused typed parameters with `_ = _ctx;` / `_ = _user_data;` / `_ = out_custom;`. None of
/// these are calls, so rebinding the first one produced `const result = _ctx;` — a value
/// nothing reads, which Zig 0.16 rejects as an unused local constant. Table-driven: each row
/// is a bare-identifier discard that must stay a discard.
#[test]
fn a_discarded_bare_identifier_is_not_a_rebindable_call_statement() {
    let non_call_discards = [
        "        _ = _ctx;",
        "        _ = _user_data;",
        "        _ = out_custom;",
        "        _ = out_len;",
        "        _ = _level;",
    ];
    for line in non_call_discards {
        assert_eq!(discarded_call_statement(line), None, "line: {line}");
    }
}

/// Positive control paired with the negative table above: a genuine call discard must still
/// be recognised as rebindable, proving the `(...)` requirement excludes bare identifiers
/// without also excluding real calls.
#[test]
fn a_discarded_call_is_still_recognised_as_rebindable() {
    let call_discards = [
        (
            "    _ = try htmd.convert(allocator, html, null);",
            "try htmd.convert(allocator, html, null);",
        ),
        (
            "_ = htmd.convert(allocator, html, null);",
            "htmd.convert(allocator, html, null);",
        ),
        ("    _ = sample.count();", "sample.count();"),
    ];
    for (line, expected) in call_discards {
        assert_eq!(discarded_call_statement(line), Some(expected), "line: {line}");
    }
}
