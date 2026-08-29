//! Shared Python helper functions emitted into a generated test file's own body.
//!
//! Neither helper is importable from a shared module — `conftest.py` carries pytest fixtures,
//! not importable helpers — so every generated file that calls one must also define it.

use std::fmt::Write as FmtWrite;

/// Emit `_alef_e2e_text`, the scalar coercion both the enum `equals` assertion and
/// `_alef_e2e_item_texts` call.
pub(crate) fn render_text_helper(out: &mut String) {
    let _ = writeln!(out, "def _alef_e2e_text(value: object) -> str:");
    let _ = writeln!(out, "    return \"\" if value is None else str(value)");
    let _ = writeln!(out);
    let _ = writeln!(out);
}

/// Emit `_alef_e2e_item_texts`, which the array `contains`/`contains_any` assertions call.
/// Its body references `_alef_e2e_text`, so [`render_text_helper`] must run alongside it.
pub(crate) fn render_item_texts_helper(out: &mut String) {
    let _ = writeln!(out, "def _alef_e2e_item_texts(item: object) -> tuple[str, ...]:");
    let _ = writeln!(out, "    raw_items = getattr(item, \"items\", None)");
    let _ = writeln!(
        out,
        "    items_text = \" \".join(str(value) for value in raw_items) if isinstance(raw_items, list) else \"\""
    );
    let _ = writeln!(out, "    return (");
    let _ = writeln!(out, "        _alef_e2e_text(item),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"kind\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"name\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"source\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"alias\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"text\", None)),");
    let _ = writeln!(out, "        _alef_e2e_text(getattr(item, \"signature\", None)),");
    let _ = writeln!(out, "        items_text,");
    let _ = writeln!(out, "    )");
    let _ = writeln!(out);
    let _ = writeln!(out);
}
