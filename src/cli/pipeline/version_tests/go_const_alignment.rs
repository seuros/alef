//! `cmd/setup/main.go`'s `versionIdent` const is emitted inside a gofmt-aligned `const (...)`
//! block, then rewritten in place by [`sync_go_cmd_setup_version_ident`]. Two different pieces of
//! code therefore decide what that line looks like: the template owns the alignment column, the
//! rewriter owns the quoted value. When the rewriter's replacement hard-coded a single space
//! around `=`, every regenerated consumer shipped a `cmd/setup/main.go` that failed `gofmt -l`
//! (observed in tree-sitter-language-pack at alef 0.63.0) — a lint failure with no local cause,
//! because the template it was generated from was correctly aligned.
//!
//! These tests pin both halves of that contract against the REAL template text rather than a
//! hand-copied fixture, so the pair cannot drift apart again.

use super::*;

/// The exact `const (...)` block `cmd_setup_main.go.jinja` emits, with the Jinja placeholders
/// left in place — the alignment under test is a property of the literal template text, not of
/// any particular rendered value.
const CMD_SETUP_TEMPLATE: &str = include_str!("../../../backends/go/templates/cmd_setup_main.go.jinja");

/// Column of the `=` in a `\tname<pad>= "value"` const line, or `None` if the line is not one.
fn const_assignment_column(line: &str) -> Option<usize> {
    let trimmed = line.strip_prefix('\t')?;
    if !trimmed.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let equals = trimmed.find('=')?;
    trimmed[..equals].split_whitespace().count().eq(&1).then_some(equals)
}

/// The lines of the `const (...)` block that declares `versionIdent`. Scoping the scan to this
/// block matters: the template also contains tab-indented plain assignments elsewhere (`cmd.Dir =
/// dir`), which gofmt does not align against the const block and which a naive whole-file scan
/// would wrongly demand be aligned with it.
fn version_ident_const_block(template: &str) -> Vec<&str> {
    let mut lines = template.lines().skip_while(|line| !line.starts_with("const ("));
    lines.next();
    lines.take_while(|line| !line.starts_with(')')).collect()
}

/// Every `=` in the template's const block must sit at one shared column — this is what `gofmt`
/// enforces, and what the rewriter must not disturb. If this fails, the template itself drifted
/// and the sibling test's premise is void.
#[test]
fn cmd_setup_template_const_block_is_gofmt_aligned() {
    let columns: Vec<(usize, &str)> = version_ident_const_block(CMD_SETUP_TEMPLATE)
        .into_iter()
        .filter_map(|line| const_assignment_column(line).map(|column| (column, line)))
        .collect();

    assert!(
        columns.len() >= 5,
        "expected the cmd/setup const block to be found in the template; got {} aligned lines",
        columns.len()
    );
    let (first_column, first_line) = columns[0];
    for (column, line) in &columns {
        assert_eq!(
            *column, first_column,
            "cmd_setup_main.go.jinja's const block must be gofmt-aligned:\n  {first_line}\n  {line}"
        );
    }
    assert!(
        columns.iter().any(|(_, line)| line.contains("versionIdent")),
        "versionIdent must be one of the aligned const lines"
    );
}

/// The rewriter must replay the alignment padding it found instead of collapsing it. Asserted
/// against the template's own `versionIdent` line so this test cannot pass on a fixture that no
/// longer resembles what alef emits.
#[test]
fn sync_go_cmd_setup_version_ident_preserves_gofmt_alignment() {
    let rendered = CMD_SETUP_TEMPLATE.replace("{{ version_ident }}", "1_15_5");
    let before = rendered
        .lines()
        .find(|line| line.contains("versionIdent"))
        .expect("template declares versionIdent")
        .to_string();

    let updated = sync_go_cmd_setup_version_ident(&rendered, "1_15_6").expect("value changed, so a rewrite happened");
    let after = updated
        .lines()
        .find(|line| line.contains("versionIdent"))
        .expect("versionIdent survives the rewrite")
        .to_string();

    assert_eq!(
        after,
        before.replace("1_15_5", "1_15_6"),
        "only the quoted value may change; the alignment padding around `=` must be replayed \
         verbatim, or the regenerated cmd/setup/main.go fails `gofmt -l`"
    );
    assert_eq!(
        const_assignment_column(&after),
        const_assignment_column(&before),
        "the `=` column must not move"
    );
}

/// Idempotence is what makes `alef generate` safe to re-run: a second sync over already-current
/// content must report "nothing changed" rather than rewriting (and thereby re-collapsing) the line.
#[test]
fn sync_go_cmd_setup_version_ident_is_idempotent_on_current_content() {
    let rendered = CMD_SETUP_TEMPLATE.replace("{{ version_ident }}", "1_15_6");

    assert_eq!(
        sync_go_cmd_setup_version_ident(&rendered, "1_15_6"),
        None,
        "content already carrying the target identifier must be left untouched"
    );
}
